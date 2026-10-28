use super::DockController;
use crate::dock::resize_session::StartGeometry;
use crate::requests::ResizeHandle;
use dock_core::{
    Axis, ColumnMode, DockCommand, DockSide, DragDropAction, DragDropRequest, DragHandle,
    DragPayload, DropTarget, Node, NodeId, Point, Rect, Size,
};
use slotmap::Key;
use ui::Theme;

fn horizontal_split_widths(controller: &DockController, split_id: NodeId) -> Vec<f32> {
    let Some(Node::Split(split)) = controller.layout().node(split_id) else {
        panic!("expected a horizontal split");
    };
    split
        .children
        .iter()
        .map(|child| match controller.layout().node(*child) {
            Some(Node::Column(column)) => column.width,
            other => panic!("horizontal split child should be a column, got {other:?}"),
        })
        .collect()
}

#[test]
fn initial_render_layout_is_valid() {
    let controller =
        DockController::new_initial(&Theme::default()).expect("initial layout should be valid");
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    assert_eq!(controller.layout().panel_instances().count(), 4);
}

#[test]
fn frame_syncs_computed_canvas_bounds_without_advancing_layout_version() {
    let mut controller =
        DockController::new_initial(&Theme::default()).expect("initial layout should be valid");
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let initial_version = controller.layout().version();

    let frame = controller.frame(viewport).unwrap();

    assert_eq!(
        controller.layout().canvas().last_known_bounds(),
        frame.rects().canvas
    );
    assert_eq!(controller.layout().version(), initial_version);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn frame_recovers_oversized_main_dock_by_collapsing_columns_to_rails() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(480.0, 800.0));

    let frame = controller
        .frame(viewport)
        .expect("a narrow viewport should recover by collapsing main columns");

    assert!(
        frame.rects().canvas.unwrap().size.width >= controller.metrics().min_canvas_width,
        "recovered layout should keep the canvas at or above its minimum width"
    );
    assert!(
        controller.layout().nodes().any(
            |(_, node)| matches!(node, Node::Column(column) if column.mode == ColumnMode::IconsOnly)
        ),
        "recovery should preserve the panel tree and make an outer column an icon rail"
    );
    assert_eq!(controller.layout().panel_instances().count(), 4);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn command_that_would_overflow_viewport_is_rejected_without_mutating_layout() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(480.0, 800.0));
    controller.frame(viewport).unwrap();

    let area = controller
        .layout()
        .main_area(DockSide::Right)
        .expect("initial layout should have a right main area");
    let column_count = match controller.layout().node(area.root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) => split.children.len(),
        other => panic!("right main area should contain columns, got {other:?}"),
    };
    let group = controller
        .layout()
        .nodes()
        .find_map(|(_, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => column.groups.first().copied(),
            _ => None,
        })
        .expect("initial layout should contain a column with multiple groups");
    let before = layout_debug_snapshot(&controller);

    let result = controller.apply(DockCommand::MoveGroupToMainColumn {
        group,
        side: DockSide::Right,
        index: column_count,
    });

    assert!(
        result.is_err(),
        "an extra column must not make the canvas too narrow"
    );
    assert_eq!(layout_debug_snapshot(&controller), before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
}

#[test]
fn history_mutations_cancel_active_resize_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let frame = controller.frame(viewport).unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let boundary_index = splitter.boundary_index;
    let column = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column,
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let group_count = column.groups.len();
    let baseline_ratios = column.group_ratios.clone();
    let column_rect = frame
        .rects()
        .columns
        .iter()
        .find(|column| column.column == owner)
        .expect("splitter column should have geometry");
    let available_height = (column_rect.rect.size.height
        - controller.metrics().splitter_thickness * (group_count - 1) as f32)
        .max(0.0);
    let handle = ResizeHandle::Splitter {
        owner,
        boundary_index,
        axis: Axis::Y,
    };
    let start = Point::new(180.0, 240.0);

    controller.begin_resize(handle, start, &frame).unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();
    assert!(controller.resize_session().is_some());

    controller
        .apply(DockCommand::ResizeGroups {
            column: owner,
            boundary_index,
            delta: 12.0,
            available_height,
        })
        .unwrap();
    assert!(controller.resize_session().is_none());
    let applied_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("resize owner should remain a column, got {other:?}"),
    };
    assert_ne!(applied_ratios, baseline_ratios);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("resize owner should remain a column, got {other:?}"),
        },
        baseline_ratios
    );

    let frame = controller.frame(viewport).unwrap();
    controller.begin_resize(handle, start, &frame).unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 5.0))
        .unwrap();
    assert!(controller.redo().unwrap());
    assert!(controller.resize_session().is_none());
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("resize owner should remain a column, got {other:?}"),
        },
        applied_ratios
    );

    let frame = controller.frame(viewport).unwrap();
    controller.begin_resize(handle, start, &frame).unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 5.0))
        .unwrap();
    assert!(controller.undo().unwrap());
    assert!(controller.resize_session().is_none());
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("resize owner should remain a column, got {other:?}"),
        },
        baseline_ratios
    );
}

#[test]
fn begin_resize_rejects_non_finite_pointer_without_starting_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let baseline_version = controller.layout().version();

    assert_eq!(
        controller.begin_resize(
            ResizeHandle::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::Y,
            },
            Point::new(f32::NAN, 240.0),
            &frame,
        ),
        Err(super::super::resize_session::ResizeStartError::InvalidPointer)
    );
    assert!(controller.resize_session().is_none());
    assert_eq!(controller.layout().version(), baseline_version);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn begin_resize_rejects_stale_frame_geometry() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let frame = controller.frame(viewport).unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let column_rect = frame
        .rects()
        .columns
        .iter()
        .find(|column| column.column == splitter.owner)
        .expect("splitter column should have geometry");
    let group_count = match controller.layout().node(splitter.owner) {
        Some(Node::Column(column)) => column.groups.len(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let available_height = column_rect.rect.size.height
        - controller.metrics().splitter_thickness * (group_count - 1) as f32;
    let handle = ResizeHandle::Splitter {
        owner: splitter.owner,
        boundary_index: splitter.boundary_index,
        axis: Axis::Y,
    };

    controller
        .apply(DockCommand::ResizeGroups {
            column: splitter.owner,
            boundary_index: splitter.boundary_index,
            delta: 12.0,
            available_height,
        })
        .unwrap();
    let current_version = controller.layout().version();
    assert_ne!(frame.layout_version(), current_version);

    assert_eq!(
        controller.begin_resize(handle, Point::new(180.0, 240.0), &frame),
        Err(super::super::resize_session::ResizeStartError::StaleFrameGeometry)
    );
    assert!(controller.resize_session().is_none());
    assert_eq!(controller.layout().version(), current_version);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn begin_resize_captures_start_geometry_and_rejects_a_second_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let frame = controller.frame(viewport).unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("the initial layout should contain a vertical group splitter");
    let group_count = match controller.layout().node(splitter.owner) {
        Some(Node::Column(column)) => column.groups.len(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let column_rect = frame
        .rects()
        .columns
        .iter()
        .find(|column| column.column == splitter.owner)
        .expect("splitter column should have geometry");
    let expected_available_height = (column_rect.rect.size.height
        - controller.metrics().splitter_thickness * (group_count - 1) as f32)
        .max(0.0);
    let handle = ResizeHandle::Splitter {
        owner: splitter.owner,
        boundary_index: splitter.boundary_index,
        axis: Axis::Y,
    };
    let pointer = Point::new(180.0, 240.0);

    controller.begin_resize(handle, pointer, &frame).unwrap();

    let session = controller.resize_session().unwrap();
    assert_eq!(session.handle(), handle);
    assert_eq!(session.pointer_start(), pointer);
    assert_eq!(
        session.start_geometry(),
        StartGeometry::VerticalGroups {
            available_height: expected_available_height,
        }
    );
    assert_eq!(session.baseline().panel_instances().count(), 4);
    assert_eq!(
        controller.begin_resize(handle, pointer, &frame),
        Err(super::super::resize_session::ResizeStartError::SessionAlreadyActive)
    );
}

#[test]
fn floating_edge_resize_commits_and_undoes_as_one_gesture() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let group = controller
        .layout()
        .nodes()
        .find_map(|(_, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => column.groups.first().copied(),
            _ => None,
        })
        .expect("initial layout should contain a column with multiple groups");
    controller
        .apply(DockCommand::Float {
            node: group,
            bounds: Rect::new(Point::new(80.0, 60.0), Size::new(500.0, 400.0)),
        })
        .unwrap();
    let (floating_id, original_bounds) = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, floating)| (id, floating.bounds))
        .expect("Float should create a floating window");
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let edge = *frame
        .rects()
        .floating_resize_edges
        .iter()
        .find(|entry| entry.group == floating_id && entry.edge == dock_core::ResizeEdge::Right)
        .expect("floating window should expose a right resize edge");
    let start = Point::new(edge.rect.origin.x, edge.rect.origin.y);
    let end = Point::new(start.x + 20.0, start.y);

    controller
        .begin_resize(
            ResizeHandle::FloatingEdge {
                group: floating_id,
                edge: dock_core::ResizeEdge::Right,
            },
            start,
            &frame,
        )
        .unwrap();
    controller.update_resize(end).unwrap();
    assert_ne!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        original_bounds
    );
    controller.commit_resize(end).unwrap();
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    assert!(controller.undo().unwrap());
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        original_bounds
    );
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn outer_column_resize_commits_and_undoes_on_both_sides() {
    for side in [DockSide::Left, DockSide::Right] {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();

        // The initial app layout only guarantees a right-side main area.
        // Build the missing left-side area directly in the test fixture so
        // setup does not add an unrelated undo-history entry.
        if side == DockSide::Left {
            let source_column =
                crate::tree_query::main_column_ids(controller.layout(), DockSide::Right)
                    .first()
                    .copied()
                    .expect("initial layout should expose a source right column");
            let group = match controller.layout().node(source_column) {
                Some(Node::Column(value)) => value.groups.first().copied(),
                other => panic!("source main column should be a column, got {other:?}"),
            }
            .expect("source right column should contain a group");

            controller
                .layout
                .apply(
                    DockCommand::MoveGroupToMainColumn {
                        group,
                        side: DockSide::Left,
                        index: 0,
                    },
                    &controller.metrics,
                )
                .expect("test setup should create a left main column");
        }

        let columns = crate::tree_query::main_column_ids(controller.layout(), side);
        let column = match side {
            DockSide::Left => columns.last().copied(),
            DockSide::Right => columns.first().copied(),
        }
        .expect("initial layout should expose an outer column");
        assert!(matches!(
            controller.layout().node(column),
            Some(Node::Column(value)) if !value.mode.is_rail()
        ));
        let original_width = match controller.layout().node(column) {
            Some(Node::Column(value)) => value.width,
            other => panic!("outer resize target should be a column, got {other:?}"),
        };
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let column_rect = frame
            .rects()
            .columns
            .iter()
            .find(|entry| entry.column == column)
            .expect("outer column should have frame geometry");
        let start = Point::new(column_rect.rect.origin.x, column_rect.rect.origin.y);
        let end = Point::new(start.x + 16.0, start.y);

        controller
            .begin_resize(ResizeHandle::OuterColumn { column, side }, start, &frame)
            .unwrap();
        controller.update_resize(end).unwrap();
        let preview_width = match controller.layout().node(column) {
            Some(Node::Column(value)) => value.width,
            _ => panic!("outer resize target should remain a column"),
        };
        assert_ne!(
            preview_width, original_width,
            "{side:?} preview must resize"
        );
        controller.commit_resize(end).unwrap();
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        assert!(controller.undo().unwrap());
        assert_eq!(
            match controller.layout().node(column) {
                Some(Node::Column(value)) => value.width,
                _ => panic!("outer resize target should remain a column after undo"),
            },
            original_width
        );
        assert!(
            !controller.undo().unwrap(),
            "one outer resize must create one undo step"
        );
    }
}

#[test]
fn horizontal_main_split_resize_commits_and_undoes_as_one_gesture() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let owner = controller
        .layout()
        .main_area(DockSide::Right)
        .map(|area| area.root)
        .expect("initial layout should have a right main area");
    assert!(matches!(
        controller.layout().node(owner),
        Some(Node::Split(split)) if split.axis == Axis::X && split.children.len() >= 2
    ));
    let original_widths = horizontal_split_widths(&controller, owner);
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|entry| entry.owner == owner && entry.axis == Axis::X)
        .expect("main horizontal split should have splitter geometry");
    let start = Point::new(splitter.rect.origin.x, splitter.rect.origin.y);
    let end = Point::new(start.x + 16.0, start.y);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::X,
            },
            start,
            &frame,
        )
        .unwrap();
    controller.update_resize(end).unwrap();
    assert_ne!(horizontal_split_widths(&controller, owner), original_widths);
    controller.commit_resize(end).unwrap();
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    assert!(controller.undo().unwrap());
    assert_eq!(horizontal_split_widths(&controller, owner), original_widths);
    assert!(
        !controller.undo().unwrap(),
        "one resize must create one undo step"
    );
}

#[test]
fn floating_horizontal_split_resize_commits_and_undoes_as_one_gesture() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let owner = controller
        .layout()
        .main_area(DockSide::Right)
        .map(|area| area.root)
        .expect("initial layout should have a right main area");
    assert!(matches!(
        controller.layout().node(owner),
        Some(Node::Split(split)) if split.axis == Axis::X && split.children.len() >= 2
    ));
    controller
        .apply(DockCommand::Float {
            node: owner,
            bounds: Rect::new(Point::new(80.0, 60.0), Size::new(700.0, 560.0)),
        })
        .unwrap();
    let floating_id = controller
        .layout()
        .floating_groups()
        .find_map(|(id, floating)| {
            controller
                .layout()
                .dock_area(floating.area)
                .is_some_and(|area| area.root == owner)
                .then_some(id)
        })
        .expect("horizontal split should now be a floating root");
    assert!(controller.layout().main_area(DockSide::Right).is_none());
    let original_widths = horizontal_split_widths(&controller, owner);
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|entry| entry.owner == owner && entry.axis == Axis::X)
        .expect("floating horizontal split should have splitter geometry");
    let start = Point::new(splitter.rect.origin.x, splitter.rect.origin.y);
    let end = Point::new(start.x + 16.0, start.y);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::X,
            },
            start,
            &frame,
        )
        .unwrap();
    controller.update_resize(end).unwrap();
    assert_ne!(horizontal_split_widths(&controller, owner), original_widths);
    controller.commit_resize(end).unwrap();
    assert!(controller.layout().floating_group(floating_id).is_some());
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    assert!(controller.undo().unwrap());
    assert!(controller.layout().floating_group(floating_id).is_some());
    assert_eq!(horizontal_split_widths(&controller, owner), original_widths);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn committing_resize_records_one_undo_step_for_the_whole_gesture() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let start = Point::new(180.0, 240.0);
    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();

    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();
    let preview_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        _ => panic!("resize owner should remain a column"),
    };
    controller
        .commit_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();

    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column"),
        },
        preview_ratios
    );
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    assert!(controller.resize_session().is_none());
    assert!(controller.undo().unwrap());
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after undo"),
        },
        original_ratios
    );
    assert!(
        !controller.undo().unwrap(),
        "one resize should create one undo step"
    );
}

#[test]
fn committing_resize_without_movement_does_not_create_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let frame = controller.frame(viewport).unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let original_version = controller.layout().version();
    let start = Point::new(180.0, 240.0);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller.commit_resize(start).unwrap();

    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column"),
        },
        original_ratios
    );
    assert!(controller.resize_session().is_none());
    assert!(
        !controller.undo().unwrap(),
        "a zero-delta resize must not add history"
    );
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn cancelling_resize_restores_baseline_and_clears_the_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let baseline_version = controller.layout().version();
    let start = Point::new(180.0, 240.0);
    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();
    assert!(controller.cancel_resize());

    assert_eq!(controller.layout().version(), baseline_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after cancel"),
        },
        original_ratios
    );
    assert!(controller.resize_session().is_none());
    assert!(!controller.cancel_resize());
    assert!(matches!(
        controller.update_resize(start),
        Err(super::super::resize_session::ResizeUpdateError::NoActiveSession)
    ));
    assert!(!controller.undo().unwrap());
}

#[test]
fn invalid_resize_preview_preserves_last_valid_preview() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let boundary_index = splitter.boundary_index;
    let start = Point::new(180.0, 240.0);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();

    let preview_version = controller.layout().version();
    let preview_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("resize owner should remain a column, got {other:?}"),
    };

    assert!(matches!(
        controller.update_resize(Point::new(f32::NAN, start.y)),
        Err(super::super::resize_session::ResizeUpdateError::InvalidPointer)
    ));
    assert_eq!(controller.layout().version(), preview_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after rejected preview"),
        },
        preview_ratios
    );
    assert!(controller.resize_session().is_some());

    controller
        .update_resize(Point::new(start.x, start.y + 20.0))
        .unwrap();
    assert!(controller.resize_session().is_some());
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn cancelling_resize_after_rejected_preview_restores_baseline() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|entry| entry.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let boundary_index = splitter.boundary_index;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let original_version = controller.layout().version();
    let start = Point::new(180.0, 240.0);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();

    assert!(matches!(
        controller.update_resize(Point::new(f32::NAN, start.y)),
        Err(super::super::resize_session::ResizeUpdateError::InvalidPointer)
    ));
    assert!(controller.resize_session().is_some());

    assert!(controller.cancel_resize());
    assert!(controller.resize_session().is_none());
    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after cancel"),
        },
        original_ratios
    );
    assert!(
        !controller.undo().unwrap(),
        "cancelled gesture must not add history"
    );
}

#[test]
fn valid_resize_commit_after_rejected_preview_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|entry| entry.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let boundary_index = splitter.boundary_index;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let start = Point::new(180.0, 240.0);

    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();

    assert!(matches!(
        controller.update_resize(Point::new(f32::NAN, start.y)),
        Err(super::super::resize_session::ResizeUpdateError::InvalidPointer)
    ));
    assert!(controller.resize_session().is_some());

    controller
        .commit_resize(Point::new(start.x, start.y + 24.0))
        .unwrap();
    assert!(controller.resize_session().is_none());
    assert_ne!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after commit"),
        },
        original_ratios
    );
    assert!(controller.undo().unwrap());
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after undo"),
        },
        original_ratios
    );
    assert!(
        !controller.undo().unwrap(),
        "gesture should add exactly one undo step"
    );
}

#[test]
fn failed_resize_commit_restores_baseline_and_clears_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let boundary_index = splitter.boundary_index;
    let original_ratios = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let baseline_version = controller.layout().version();
    let start = Point::new(180.0, 240.0);
    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();
    controller
        .update_resize(Point::new(start.x, start.y + 16.0))
        .unwrap();

    assert!(matches!(
        controller.commit_resize(Point::new(f32::NAN, start.y)),
        Err(super::super::resize_session::ResizeUpdateError::InvalidPointer)
    ));
    assert_eq!(controller.layout().version(), baseline_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column after failed commit"),
        },
        original_ratios
    );
    assert!(controller.resize_session().is_none());
    assert!(!controller.undo().unwrap());
}

#[test]
fn resize_preview_is_recomputed_from_baseline_and_zero_delta_restores_it() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let frame = controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();
    let splitter = *frame
        .rects()
        .splitters
        .iter()
        .find(|splitter| splitter.axis == Axis::Y)
        .expect("initial layout should contain a vertical group splitter");
    let owner = splitter.owner;
    let original = match controller.layout().node(owner) {
        Some(Node::Column(column)) => column.group_ratios.clone(),
        other => panic!("vertical splitter should belong to a column, got {other:?}"),
    };
    let initial_version = controller.layout().version();
    let start = Point::new(180.0, 240.0);
    controller
        .begin_resize(
            ResizeHandle::Splitter {
                owner,
                boundary_index: splitter.boundary_index,
                axis: Axis::Y,
            },
            start,
            &frame,
        )
        .unwrap();

    let preview = controller
        .update_resize(Point::new(start.x, start.y + 12.0))
        .unwrap();
    assert_eq!(controller.layout().version(), preview.new_version);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    controller.update_resize(start).unwrap();
    assert_eq!(controller.layout().version(), initial_version);
    assert_eq!(
        match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column"),
        },
        original
    );
}

fn group_containing_panel(
    controller: &DockController,
    panel: dock_core::PanelId,
) -> Option<NodeId> {
    controller.layout().nodes().find_map(|(group, node)| {
        let Node::Tabs(tabs) = node else {
            return None;
        };
        tabs.children
            .iter()
            .any(|child| {
                matches!(
                    controller.layout().node(*child),
                    Some(Node::Panel(candidate)) if candidate.panel_id == panel
                )
            })
            .then_some(group)
    })
}

fn first_panels_by_group(controller: &DockController) -> Vec<(NodeId, NodeId, dock_core::PanelId)> {
    controller
        .layout()
        .nodes()
        .filter_map(|(group, node)| {
            let Node::Tabs(tabs) = node else {
                return None;
            };
            let panel_node = *tabs.children.first()?;
            let Some(Node::Panel(panel)) = controller.layout().node(panel_node) else {
                return None;
            };
            Some((group, panel_node, panel.panel_id))
        })
        .collect()
}

fn owner_column(controller: &DockController, group: NodeId) -> Option<NodeId> {
    controller
        .layout()
        .nodes()
        .find_map(|(column_id, node)| match node {
            Node::Column(column) if column.groups.contains(&group) => Some(column_id),
            _ => None,
        })
}

fn make_initial_floating_group(
    controller: &mut DockController,
) -> (dock_core::FloatingGroupId, Rect) {
    let group = controller
        .layout()
        .nodes()
        .find_map(|(_, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => column.groups.first().copied(),
            _ => None,
        })
        .expect("initial layout should have a column with multiple groups");
    let initial_bounds = Rect::new(Point::new(80.0, 60.0), Size::new(500.0, 400.0));
    controller
        .apply(DockCommand::Float {
            node: group,
            bounds: initial_bounds,
        })
        .unwrap();
    let (floating_id, bounds) = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, floating)| (id, floating.bounds))
        .expect("Float should create a floating window");

    // Setup is outside the drag's history assertion.
    controller.history.clear();
    (floating_id, bounds)
}

fn start_active_floating_title_drag(
    controller: &mut DockController,
    viewport: Rect,
) -> (dock_core::FloatingGroupId, Rect, Point) {
    let (floating_id, bounds) = make_initial_floating_group(controller);
    let frame = controller.frame(viewport).unwrap();
    let origin = Point::new(
        bounds.origin.x + 48.0,
        bounds.origin.y + controller.metrics().header_height / 2.0,
    );
    let threshold = controller.metrics().drag_threshold;
    controller
        .begin_drag(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::FloatingTitle,
            origin,
        )
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x + threshold + 20.0, origin.y + 12.0),
                viewport,
                Some(&frame),
            )
            .unwrap(),
        "moving beyond the threshold should activate the floating title drag"
    );
    (floating_id, bounds, origin)
}

fn drop_request(
    panel: dock_core::PanelId,
    target: DropTarget,
    origin: Point,
    point: Point,
) -> DragDropRequest {
    DragDropRequest {
        payload: DragPayload::Panel { panel },
        handle: DragHandle::PanelTab,
        origin,
        point,
        target: Some(target),
        floating_bounds: None,
    }
}

fn layout_debug_snapshot(controller: &DockController) -> String {
    format!("{:?}", controller.layout())
}

fn start_active_panel_drag(
    controller: &mut DockController,
    viewport: Rect,
) -> (dock_core::PanelId, Point) {
    let (_, _, panel) = first_panels_by_group(controller)[0];
    let frame = controller.frame(viewport).unwrap();
    let tab_rect = frame
        .rects()
        .tab_headers
        .iter()
        .find(|header| header.panel == panel)
        .map(|header| header.rect)
        .expect("initial panel should have a visible tab header");
    let origin = Point::new(
        tab_rect.origin.x + tab_rect.size.width / 2.0,
        tab_rect.origin.y + tab_rect.size.height / 2.0,
    );
    let threshold = controller.metrics().drag_threshold;

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x + threshold + 20.0, origin.y),
                viewport,
                Some(&frame),
            )
            .unwrap(),
        "movement beyond threshold should activate the drag"
    );

    (panel, origin)
}

#[test]
fn stale_frame_before_drag_activation_cancels_pending_session() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let frame = controller.frame(viewport).unwrap();
    let (_, _, panel) = first_panels_by_group(&controller)[0];
    let tab_rect = frame
        .rects()
        .tab_headers
        .iter()
        .find(|header| header.panel == panel)
        .map(|header| header.rect)
        .expect("initial panel should have a visible tab header");
    let origin = Point::new(
        tab_rect.origin.x + tab_rect.size.width / 2.0,
        tab_rect.origin.y + tab_rect.size.height / 2.0,
    );
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    let threshold = controller.metrics().drag_threshold;
    let changed_viewport = Rect::new(Point::ZERO, Size::new(1100.0, 750.0));

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(matches!(
        controller.update_drag(
            Point::new(origin.x + threshold + 20.0, origin.y),
            changed_viewport,
            Some(&frame),
        ),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(!controller.cancel_drag());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn stale_drag_viewport_change_cancels_session_without_mutating_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (panel, origin) = start_active_panel_drag(&mut controller, viewport);
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    let smaller_viewport = Rect::new(Point::ZERO, Size::new(1000.0, 700.0));
    let pointer = Point::new(
        origin.x + controller.metrics().drag_threshold + 30.0,
        origin.y,
    );

    assert!(matches!(
        controller.update_drag(pointer, smaller_viewport, None),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(
        !controller.cancel_drag(),
        "stale geometry must end the session internally"
    );
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn stale_drag_layout_version_cancels_session_before_resolving_old_zones() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (panel, origin) = start_active_panel_drag(&mut controller, viewport);
    let original_version = controller.layout().version();
    let metrics = controller.metrics;
    controller
        .layout
        .apply(
            DockCommand::SetChrome {
                mode: dock_core::ChromeMode::DockHidden,
            },
            &metrics,
        )
        .unwrap();
    assert_ne!(controller.layout().version(), original_version);
    let changed_layout = layout_debug_snapshot(&controller);
    let changed_version = controller.layout().version();

    assert!(matches!(
        controller.update_drag(
            Point::new(origin.x + metrics.drag_threshold + 35.0, origin.y),
            viewport,
            None,
        ),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(!controller.cancel_drag());
    assert_eq!(layout_debug_snapshot(&controller), changed_layout);
    assert_eq!(controller.layout().version(), changed_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn stale_drag_metrics_cancel_session_without_mutating_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (panel, origin) = start_active_panel_drag(&mut controller, viewport);
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    controller.metrics.min_column_width += 1.0;
    let pointer = Point::new(
        origin.x + controller.metrics().drag_threshold + 30.0,
        origin.y,
    );

    assert!(matches!(
        controller.update_drag(pointer, viewport, None),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(!controller.cancel_drag());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn finish_drag_with_stale_viewport_clears_session_without_emitting_request() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (panel, origin) = start_active_panel_drag(&mut controller, viewport);
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    let stale_viewport = Rect::new(Point::ZERO, Size::new(1100.0, 750.0));

    assert!(matches!(
        controller.finish_drag(origin, stale_viewport),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(!controller.cancel_drag());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn group_grip_drop_target_matrix_maps_to_one_reversible_command() {
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));

    for target_kind in ["edge", "column", "rail", "main"] {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let main_columns = crate::tree_query::main_column_ids(controller.layout(), DockSide::Right);
        let source_column = main_columns
            .iter()
            .copied()
            .find(|column_id| {
                matches!(
                    controller.layout().node(*column_id),
                    Some(Node::Column(column)) if column.groups.len() > 1 && !column.mode.is_rail()
                )
            })
            .expect("the right main dock should contain a column with multiple groups");
        let source_group = match controller.layout().node(source_column) {
            Some(Node::Column(column)) => column.groups[0],
            _ => unreachable!("source_column was selected from main columns"),
        };
        let target_column = main_columns
                .iter()
                .copied()
                .find(|column_id| {
                    *column_id != source_column
                        && matches!(
                            controller.layout().node(*column_id),
                            Some(Node::Column(column)) if !column.mode.is_rail() && !column.groups.is_empty()
                        )
                })
                .expect("a second main-dock column should provide a group target");
        let target_group = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups[0],
            _ => unreachable!("target_column was selected from main columns"),
        };
        let target_column_len = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups.len(),
            _ => unreachable!("target_column was selected from main columns"),
        };

        if target_kind == "rail" {
            controller
                .apply(DockCommand::SetColumnMode {
                    column: target_column,
                    mode: ColumnMode::IconsOnly,
                })
                .unwrap();
            // The rail mode is fixture setup; isolate history to the drop under test.
            controller.history.clear();
        }

        let target = match target_kind {
            "edge" => DropTarget::GroupEdge {
                group: target_group,
                zone: dock_core::DropZone::Right,
            },
            "column" => DropTarget::ColumnSlot {
                column: target_column,
                index: target_column_len,
            },
            "rail" => DropTarget::RailSlot {
                column: target_column,
                index: 0,
            },
            "main" => DropTarget::MainColumn {
                side: DockSide::Right,
                index: 0,
            },
            _ => unreachable!("all group target cases are listed above"),
        };

        let action = controller
            .commit_drop_request(
                DragDropRequest {
                    payload: DragPayload::Group {
                        group: source_group,
                    },
                    handle: DragHandle::GroupGrip,
                    origin: Point::new(180.0, 120.0),
                    point: Point::new(420.0, 260.0),
                    target: Some(target),
                    floating_bounds: None,
                },
                viewport,
            )
            .unwrap();

        match target_kind {
            "edge" => assert_eq!(
                action,
                DragDropAction::Command(DockCommand::SplitGroup {
                    group: source_group,
                    target_group,
                    zone: dock_core::DropZone::Right,
                    ratio: 0.5,
                })
            ),
            "column" => assert_eq!(
                action,
                DragDropAction::Command(DockCommand::MoveGroup {
                    group: source_group,
                    column: target_column,
                    index: target_column_len,
                })
            ),
            "rail" => match action {
                DragDropAction::Command(DockCommand::MoveGroup {
                    group,
                    column,
                    index,
                }) => {
                    assert_eq!(group, source_group);
                    assert_eq!(column, target_column);
                    assert!(index <= target_column_len);
                }
                other => panic!("rail-slot group drop should resolve to MoveGroup, got {other:?}"),
            },
            "main" => assert_eq!(
                action,
                DragDropAction::Command(DockCommand::MoveGroupToMainColumn {
                    group: source_group,
                    side: DockSide::Right,
                    index: 0,
                })
            ),
            _ => unreachable!("all group target cases are listed above"),
        }

        assert_eq!(
            controller.history.undo_len(),
            1,
            "one drop must be one undo step"
        );
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        match target_kind {
            "column" | "rail" => {
                assert_eq!(owner_column(&controller, source_group), Some(target_column));
            }
            "main" => {
                assert_ne!(owner_column(&controller, source_group), Some(source_column));
                assert_eq!(
                    crate::tree_query::main_column_ids(controller.layout(), DockSide::Right).len(),
                    main_columns.len() + 1
                );
            }
            "edge" => assert!(owner_column(&controller, source_group).is_some()),
            _ => unreachable!("all group target cases are listed above"),
        }

        assert!(controller.undo().unwrap());
        assert_eq!(owner_column(&controller, source_group), Some(source_column));
        assert_eq!(controller.history.undo_len(), 0);
        assert_eq!(controller.history.redo_len(), 1);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        assert!(controller.redo().unwrap());
        assert_eq!(controller.history.undo_len(), 1);
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        match target_kind {
            "column" | "rail" => {
                assert_eq!(owner_column(&controller, source_group), Some(target_column));
            }
            "main" => assert_ne!(owner_column(&controller, source_group), Some(source_column)),
            "edge" => assert!(owner_column(&controller, source_group).is_some()),
            _ => unreachable!("all group target cases are listed above"),
        }
    }
}

#[test]
fn group_grip_no_op_preserves_existing_redo_branch() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let group = first_panels_by_group(&controller)[0].0;
    let group_len = match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => unreachable!("source group should be a Tabs node"),
    };

    controller
        .apply(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::DockHidden,
        })
        .unwrap();
    assert!(controller.undo().unwrap());
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);

    let before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    let action = controller
        .commit_drop_request(
            DragDropRequest {
                payload: DragPayload::Group { group },
                handle: DragHandle::GroupGrip,
                origin: Point::new(180.0, 120.0),
                point: Point::new(240.0, 180.0),
                target: Some(DropTarget::GroupTabs {
                    group,
                    index: group_len,
                }),
                floating_bounds: None,
            },
            viewport,
        )
        .unwrap();

    assert_eq!(action, DragDropAction::NoOp);
    assert_eq!(layout_debug_snapshot(&controller), before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert!(controller.redo().unwrap());
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn group_grip_drop_merges_whole_group_and_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let source_group = controller
        .layout()
        .nodes()
        .find_map(|(_, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => column.groups.first().copied(),
            _ => None,
        })
        .expect("initial layout should have a column with multiple groups");
    let target_group = first_panels_by_group(&controller)
        .into_iter()
        .map(|(group, _, _)| group)
        .find(|group| *group != source_group)
        .expect("initial layout should contain a different target group");
    let source_panel_ids: Vec<_> = match controller.layout().node(source_group) {
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .filter_map(|node| match controller.layout().node(*node) {
                Some(Node::Panel(panel)) => Some(panel.panel_id),
                _ => None,
            })
            .collect(),
        _ => panic!("group-grip source should be a Tabs node"),
    };
    assert!(!source_panel_ids.is_empty());
    let target_index = match controller.layout().node(target_group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => panic!("target group should be a Tabs node"),
    };
    let original_version = controller.layout().version();
    let original_floating_count = controller.layout().floating_groups().count();

    let action = controller
        .commit_drop_request(
            DragDropRequest {
                payload: DragPayload::Group {
                    group: source_group,
                },
                handle: DragHandle::GroupGrip,
                origin: Point::new(180.0, 120.0),
                point: Point::new(420.0, 260.0),
                target: Some(DropTarget::GroupTabs {
                    group: target_group,
                    index: target_index,
                }),
                floating_bounds: None,
            },
            viewport,
        )
        .unwrap();

    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MergeGroup {
            group: source_group,
            target_group,
            index: target_index,
        })
    );
    assert_ne!(controller.layout().version(), original_version);
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count
    );
    assert_eq!(controller.history.undo_len(), 1);
    for panel in &source_panel_ids {
        assert_eq!(
            group_containing_panel(&controller, *panel),
            Some(target_group)
        );
    }
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    for panel in &source_panel_ids {
        assert_ne!(
            group_containing_panel(&controller, *panel),
            Some(target_group),
            "undo must restore each source panel outside the merged target"
        );
    }
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.redo().unwrap());
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    for panel in &source_panel_ids {
        assert_eq!(
            group_containing_panel(&controller, *panel),
            Some(target_group)
        );
    }
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_capture_drag_resolves_dock_target_and_docks_entire_window() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, bounds) = make_initial_floating_group(&mut controller);
    let frame = controller.frame(viewport).unwrap();
    let floating_root = controller
        .layout()
        .floating_group(floating_id)
        .and_then(|floating| controller.layout().dock_area(floating.area))
        .map(|area| area.root)
        .expect("floating group should have a root");
    let source_nodes = crate::tree_query::subtree_nodes(controller.layout(), floating_root);
    let capture_width = controller.metrics().header_height.min(28.0);
    let origin = Point::new(
        bounds.origin.x + bounds.size.width - capture_width / 2.0,
        bounds.origin.y + controller.metrics().header_height / 2.0,
    );
    let threshold = controller.metrics().drag_threshold;

    controller
        .begin_drag(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::FloatingCapture,
            origin,
        )
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x - threshold - 20.0, origin.y + 8.0),
                viewport,
                Some(&frame),
            )
            .unwrap(),
        "capture movement should activate a drag with a resolver"
    );

    let overlay = controller
        .drag_overlay()
        .expect("floating capture must show docking zones");
    assert_eq!(overlay.handle, DragHandle::FloatingCapture);
    assert!(
        overlay
            .zones
            .iter()
            .all(|zone| zone.target != DropTarget::FreeArea),
        "an already floating window must not offer FreeArea as a capture target"
    );
    assert!(
        overlay.zones.iter().all(|zone| {
            crate::tree_query::drop_target_node(zone.target)
                .is_none_or(|target| !source_nodes.contains(&target))
        }),
        "the capture overlay must not offer targets inside its own floating tree"
    );

    let target_group = first_panels_by_group(&controller)
        .into_iter()
        .map(|(group, _, _)| group)
        .find(|group| !source_nodes.contains(group))
        .expect("a main-dock group should remain as a docking destination");
    let zone = overlay
        .zones
        .iter()
        .find(|zone| {
            zone.zone == dock_core::DropZone::Center
                && matches!(
                    zone.target,
                    DropTarget::GroupTabs { group, .. } if group == target_group
                )
        })
        .copied()
        .expect("capture resolver should offer merging into a main-dock group");
    let release = Point::new(
        zone.rect.origin.x + zone.rect.size.width / 2.0,
        zone.rect.origin.y + zone.rect.size.height / 2.0,
    );

    controller.update_drag(release, viewport, None).unwrap();
    assert_eq!(
        controller.drag_overlay().unwrap().preview.target,
        Some(zone.target)
    );
    let request = controller
        .finish_drag(release, viewport)
        .unwrap()
        .expect("capture release over a docking zone should emit a request");
    assert_eq!(request.handle, DragHandle::FloatingCapture);
    assert_eq!(
        request.payload,
        DragPayload::FloatingGroup { group: floating_id }
    );
    assert_eq!(request.target, Some(zone.target));

    let action = controller.commit_drop_request(request, viewport).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::DockFloating {
            group: floating_id,
            target: zone.target,
        })
    );
    assert!(controller.layout().floating_group(floating_id).is_none());
    assert_eq!(controller.layout().floating_groups().count(), 0);
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    // Restored arena entries may receive fresh runtime IDs after docking removed them.
    let restored_floating = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, floating)| (id, floating.bounds))
        .expect("undo should restore one floating group");
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert_eq!(restored_floating.1, bounds);
    assert_eq!(controller.history.undo_len(), 0);
    assert!(controller.redo().unwrap());
    assert_eq!(controller.layout().floating_groups().count(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_capture_docks_a_panel_root_and_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (_, panel_node, panel) = first_panels_by_group(&controller)[0];
    controller
        .apply(DockCommand::Float {
            node: panel_node,
            bounds: Rect::new(Point::new(80.0, 60.0), Size::new(500.0, 400.0)),
        })
        .unwrap();
    controller.history.clear();

    let floating_id = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, _)| id)
        .expect("floating a panel root should create a floating window");
    let (target_group, target_index) = first_panels_by_group(&controller)
        .into_iter()
        .find_map(|(group, _, _)| match controller.layout().node(group) {
            Some(Node::Tabs(tabs)) => Some((group, tabs.children.len())),
            _ => None,
        })
        .expect("a docked Tabs group should remain as a capture target");
    let target = DropTarget::GroupTabs {
        group: target_group,
        index: target_index,
    };

    let action = controller
        .commit_drop_request(
            DragDropRequest {
                payload: DragPayload::FloatingGroup { group: floating_id },
                handle: DragHandle::FloatingCapture,
                origin: Point::new(120.0, 80.0),
                point: Point::new(420.0, 260.0),
                target: Some(target),
                floating_bounds: None,
            },
            viewport,
        )
        .unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::DockFloating {
            group: floating_id,
            target,
        })
    );
    assert_eq!(controller.layout().floating_groups().count(), 0);
    assert_eq!(
        group_containing_panel(&controller, panel),
        Some(target_group)
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert_ne!(
        group_containing_panel(&controller, panel),
        Some(target_group)
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.redo().unwrap());
    assert_eq!(controller.layout().floating_groups().count(), 0);
    assert_eq!(
        group_containing_panel(&controller, panel),
        Some(target_group)
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_capture_drop_into_its_own_tree_is_no_op() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, bounds) = make_initial_floating_group(&mut controller);
    let root = controller
        .layout()
        .floating_group(floating_id)
        .and_then(|floating| controller.layout().dock_area(floating.area))
        .map(|area| area.root)
        .expect("floating window should expose a root node");
    let index = match controller.layout().node(root) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => panic!("the setup floating root should be a Tabs node"),
    };
    let before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();

    let action = controller
        .commit_drop_request(
            DragDropRequest {
                payload: DragPayload::FloatingGroup { group: floating_id },
                handle: DragHandle::FloatingCapture,
                origin: Point::new(bounds.origin.x + 16.0, bounds.origin.y + 16.0),
                point: Point::new(bounds.origin.x + 140.0, bounds.origin.y + 90.0),
                target: Some(DropTarget::GroupTabs { group: root, index }),
                floating_bounds: None,
            },
            viewport,
        )
        .unwrap();

    assert_eq!(action, DragDropAction::NoOp);
    assert_eq!(layout_debug_snapshot(&controller), before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_capture_rejects_free_area_without_mutating_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, bounds) = make_initial_floating_group(&mut controller);
    let before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();

    let result = controller.commit_drop_request(
        DragDropRequest {
            payload: DragPayload::FloatingGroup { group: floating_id },
            handle: DragHandle::FloatingCapture,
            origin: Point::new(bounds.origin.x + 16.0, bounds.origin.y + 16.0),
            point: Point::new(bounds.origin.x + 140.0, bounds.origin.y + 90.0),
            target: Some(DropTarget::FreeArea),
            floating_bounds: None,
        },
        viewport,
    );

    assert!(
        result.is_err(),
        "capture drag may only finish on a dock target"
    );
    assert_eq!(layout_debug_snapshot(&controller), before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        bounds
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert!(controller.last_error().is_some());
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_title_drag_moves_window_without_resolving_dock_targets() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, original_bounds, origin) =
        start_active_floating_title_drag(&mut controller, viewport);
    let original_version = controller.layout().version();
    assert!(
        controller.drag_overlay().is_none(),
        "a title-bar move must not display a docking overlay"
    );

    let release = Point::new(origin.x + 75.0, origin.y + 34.0);
    let request = controller
        .finish_drag(release, viewport)
        .unwrap()
        .expect("floating title release should produce a move request");
    assert_eq!(request.handle, DragHandle::FloatingTitle);
    assert_eq!(
        request.payload,
        DragPayload::FloatingGroup { group: floating_id }
    );
    assert_eq!(
        request.target, None,
        "title dragging never resolves a dock target"
    );

    let action = controller.commit_drop_request(request, viewport).unwrap();
    let expected_origin = Point::new(
        original_bounds.origin.x + release.x - origin.x,
        original_bounds.origin.y + release.y - origin.y,
    );
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveFloating {
            group: floating_id,
            origin: expected_origin,
        })
    );
    let actual_bounds = controller
        .layout()
        .floating_group(floating_id)
        .expect("floating window should remain floating")
        .bounds;
    assert_eq!(actual_bounds.origin, expected_origin);
    assert_eq!(actual_bounds.size, original_bounds.size);
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert!(controller.layout().version() > original_version);
    assert_eq!(controller.history.undo_len(), 1);
    assert!(controller.layout().validate(controller.metrics()).is_ok());

    assert!(controller.undo().unwrap());
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        original_bounds
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert!(controller.redo().unwrap());
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds
            .origin,
        expected_origin
    );
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert!(controller.layout().validate(controller.metrics()).is_ok());
}

#[test]
fn floating_title_drag_clamps_position_to_viewport_and_remains_undoable() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, original_bounds, origin) =
        start_active_floating_title_drag(&mut controller, viewport);

    let release = Point::new(-1000.0, -1000.0);
    let request = controller
        .finish_drag(release, viewport)
        .unwrap()
        .expect("floating title release should produce a move request");
    let action = controller.commit_drop_request(request, viewport).unwrap();

    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveFloating {
            group: floating_id,
            origin: Point::ZERO,
        })
    );
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        Rect::new(Point::ZERO, original_bounds.size)
    );
    let frame = controller.frame(viewport).unwrap();
    assert_eq!(
        frame
            .rects()
            .floating_groups
            .iter()
            .find(|floating| floating.group == floating_id)
            .unwrap()
            .rect,
        Rect::new(Point::ZERO, original_bounds.size),
        "persistent and rendered positions should agree after clamping"
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        original_bounds
    );
    assert!(controller.redo().unwrap());
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        Rect::new(Point::ZERO, original_bounds.size)
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    // The drag's original pointer is still the actual title-bar grab point;
    // clamping affects only the committed origin, not pointer-session state.
    assert!(origin.is_finite());
}

#[test]
fn floating_title_drag_uses_visible_position_when_saved_origin_is_offscreen() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, original_bounds) = make_initial_floating_group(&mut controller);
    let legacy_origin = Point::new(-500.0, 80.0);
    let metrics = *controller.metrics();
    controller
        .layout
        .apply(
            DockCommand::MoveFloating {
                group: floating_id,
                origin: legacy_origin,
            },
            &metrics,
        )
        .unwrap();
    controller.history.clear();

    let frame = controller.frame(viewport).unwrap();
    let visible_bounds = frame
        .rects()
        .floating_groups
        .iter()
        .find(|floating| floating.group == floating_id)
        .unwrap()
        .rect;
    assert_eq!(visible_bounds.origin, Point::new(0.0, legacy_origin.y));

    let origin = Point::new(
        visible_bounds.origin.x + 48.0,
        visible_bounds.origin.y + controller.metrics().header_height / 2.0,
    );
    let threshold = controller.metrics().drag_threshold;
    controller
        .begin_drag(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::FloatingTitle,
            origin,
        )
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x + threshold + 20.0, origin.y + 12.0),
                viewport,
                Some(&frame),
            )
            .unwrap()
    );

    let release = Point::new(origin.x + 40.0, origin.y + 20.0);
    let request = controller.finish_drag(release, viewport).unwrap().unwrap();
    let action = controller.commit_drop_request(request, viewport).unwrap();
    let expected_origin = Point::new(40.0, legacy_origin.y + 20.0);
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveFloating {
            group: floating_id,
            origin: expected_origin,
        })
    );
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        Rect::new(expected_origin, original_bounds.size)
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn floating_title_drag_with_stale_layout_snapshot_is_cancelled_before_commit() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (floating_id, original_bounds, origin) =
        start_active_floating_title_drag(&mut controller, viewport);
    let original_version = controller.layout().version();
    let metrics = controller.metrics;
    controller
        .layout
        .apply(
            DockCommand::SetChrome {
                mode: dock_core::ChromeMode::DockHidden,
            },
            &metrics,
        )
        .unwrap();
    assert_ne!(controller.layout().version(), original_version);
    let state_after_external_change = layout_debug_snapshot(&controller);
    let version_after_external_change = controller.layout().version();

    assert!(matches!(
        controller.finish_drag(Point::new(origin.x + 80.0, origin.y + 40.0), viewport,),
        Err(super::super::drag_session::DragUpdateError::StaleFrameGeometry)
    ));
    assert!(controller.drag_overlay().is_none());
    assert!(
        !controller.cancel_drag(),
        "finish_drag should clear the stale session"
    );
    assert_eq!(
        layout_debug_snapshot(&controller),
        state_after_external_change
    );
    assert_eq!(controller.layout().version(), version_after_external_change);
    assert_eq!(
        controller
            .layout()
            .floating_group(floating_id)
            .unwrap()
            .bounds,
        original_bounds,
        "the rejected title drag must not move or dock the floating window"
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    controller
        .begin_drag(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::FloatingTitle,
            origin,
        )
        .unwrap();
    assert!(controller.cancel_drag());
}

#[test]
fn successful_drop_commits_one_command_and_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let samples = first_panels_by_group(&controller);
    let (source_group, source_panel_node, panel) = samples[0];
    let target_group = samples
        .iter()
        .map(|(group, _, _)| *group)
        .find(|group| *group != source_group)
        .expect("initial layout should contain more than one tab group");
    let original_version = controller.layout().version();

    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                DropTarget::Tabs {
                    group: target_group,
                    index: 0,
                },
                Point::new(100.0, 100.0),
                Point::new(240.0, 180.0),
            ),
            viewport,
        )
        .unwrap();

    assert!(
        matches!(action, DragDropAction::Command(_)),
        "a drop into a different group should resolve to one command"
    );
    assert_ne!(controller.layout().version(), original_version);
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert!(
        matches!(controller.layout().node(target_group), Some(Node::Tabs(tabs))
                if tabs.children.contains(&source_panel_node)),
        "the moved panel should belong to the destination group"
    );

    assert!(controller.undo().unwrap());
    assert!(
        !matches!(controller.layout().node(target_group), Some(Node::Tabs(tabs))
                if tabs.children.contains(&source_panel_node)),
        "undo should restore the panel to its original group"
    );
    assert!(
        !controller.undo().unwrap(),
        "the drop must create exactly one undo step"
    );
    assert_eq!(controller.history.redo_len(), 1);
}

fn free_area_request(
    payload: DragPayload,
    handle: DragHandle,
    origin: Point,
    point: Point,
) -> DragDropRequest {
    DragDropRequest {
        payload,
        handle,
        origin,
        point,
        target: Some(DropTarget::FreeArea),
        floating_bounds: None,
    }
}

fn assert_bounds_inside(bounds: Rect, viewport: Rect) {
    assert!(bounds.origin.x >= viewport.origin.x);
    assert!(bounds.origin.y >= viewport.origin.y);
    assert!(bounds.origin.x + bounds.size.width <= viewport.origin.x + viewport.size.width);
    assert!(bounds.origin.y + bounds.size.height <= viewport.origin.y + viewport.size.height);
}

#[test]
fn panel_drop_target_matrix_maps_dock_targets_to_move_panel() {
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));

    for target_kind in ["edge", "column", "rail", "main"] {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let main_columns = crate::tree_query::main_column_ids(controller.layout(), DockSide::Right);
        let (source_group, panel) = controller
            .layout()
            .nodes()
            .find_map(|(group, node)| {
                let Node::Tabs(tabs) = node else {
                    return None;
                };
                if tabs.children.len() < 2 {
                    return None;
                }
                let panel_node = *tabs.children.first()?;
                let Node::Panel(panel) = controller.layout().node(panel_node)? else {
                    return None;
                };
                Some((group, panel.panel_id))
            })
            .expect("initial layout should contain a multi-tab source group");
        let source_column = owner_column(&controller, source_group)
            .expect("multi-tab source group should belong to a column");
        let target_column = main_columns
            .iter()
            .copied()
            .find(|column_id| *column_id != source_column)
            .expect("a different main-dock column should provide a target");
        let target_group = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups[0],
            _ => unreachable!("target_column was selected from main columns"),
        };
        let target_column_len = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups.len(),
            _ => unreachable!("target_column was selected from main columns"),
        };

        if target_kind == "rail" {
            controller
                .apply(DockCommand::SetColumnMode {
                    column: target_column,
                    mode: ColumnMode::IconsOnly,
                })
                .unwrap();
            controller.history.clear();
        }

        let target = match target_kind {
            "edge" => DropTarget::GroupEdge {
                group: target_group,
                zone: dock_core::DropZone::Right,
            },
            "column" => DropTarget::ColumnSlot {
                column: target_column,
                index: target_column_len,
            },
            "rail" => DropTarget::RailSlot {
                column: target_column,
                index: 0,
            },
            "main" => DropTarget::MainColumn {
                side: DockSide::Right,
                index: 0,
            },
            _ => unreachable!("all panel target cases are listed above"),
        };

        let action = controller
            .commit_drop_request(
                drop_request(
                    panel,
                    target,
                    Point::new(180.0, 120.0),
                    Point::new(420.0, 260.0),
                ),
                viewport,
            )
            .unwrap();
        assert_eq!(
            action,
            DragDropAction::Command(DockCommand::MovePanel { panel, target }),
            "panel-to-{target_kind} must remain a single MovePanel command"
        );
        assert_ne!(
            group_containing_panel(&controller, panel),
            Some(source_group)
        );
        assert_eq!(controller.history.undo_len(), 1);
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        assert!(controller.undo().unwrap());
        assert_eq!(
            group_containing_panel(&controller, panel),
            Some(source_group)
        );
        assert_eq!(controller.history.undo_len(), 0);
        assert_eq!(controller.history.redo_len(), 1);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        assert!(controller.redo().unwrap());
        assert_ne!(
            group_containing_panel(&controller, panel),
            Some(source_group)
        );
        assert_eq!(controller.history.undo_len(), 1);
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    }
}

#[test]
fn flyout_panel_drop_target_matrix_uses_move_panel_from_flyout() {
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));

    for target_kind in ["edge", "column", "rail", "main"] {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let main_columns = crate::tree_query::main_column_ids(controller.layout(), DockSide::Right);
        let (source_column, source_group, panel) = main_columns
            .iter()
            .copied()
            .find_map(|column_id| {
                let Node::Column(column) = controller.layout().node(column_id)? else {
                    return None;
                };
                column.groups.iter().find_map(|group| {
                    let Node::Tabs(tabs) = controller.layout().node(*group)? else {
                        return None;
                    };
                    if tabs.children.len() < 2 {
                        return None;
                    }
                    let active = *tabs.children.get(tabs.active_index)?;
                    let Node::Panel(panel) = controller.layout().node(active)? else {
                        return None;
                    };
                    Some((column_id, *group, panel.panel_id))
                })
            })
            .expect("initial main dock should contain a multi-tab group");
        let target_column = *main_columns
            .iter()
            .find(|column_id| **column_id != source_column)
            .expect("a different main-dock column should provide a target");
        let target_group = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups[0],
            _ => unreachable!("target_column was selected from main columns"),
        };
        let target_column_len = match controller.layout().node(target_column) {
            Some(Node::Column(column)) => column.groups.len(),
            _ => unreachable!("target_column was selected from main columns"),
        };

        controller
            .apply(DockCommand::SetColumnMode {
                column: source_column,
                mode: ColumnMode::IconsOnly,
            })
            .unwrap();
        if target_kind == "rail" {
            controller
                .apply(DockCommand::SetColumnMode {
                    column: target_column,
                    mode: ColumnMode::IconsOnly,
                })
                .unwrap();
        }
        // Keep the gesture under test as the only persistent history entry.
        controller.history.clear();
        controller
            .apply(DockCommand::OpenFlyout {
                column: source_column,
                panel,
            })
            .unwrap();
        assert!(controller.layout().flyout().is_some());
        assert_eq!(controller.history.undo_len(), 0);

        let target = match target_kind {
            "edge" => DropTarget::GroupEdge {
                group: target_group,
                zone: dock_core::DropZone::Right,
            },
            "column" => DropTarget::ColumnSlot {
                column: target_column,
                index: target_column_len,
            },
            "rail" => DropTarget::RailSlot {
                column: target_column,
                index: 0,
            },
            "main" => DropTarget::MainColumn {
                side: DockSide::Left,
                index: 0,
            },
            _ => unreachable!("all flyout target cases are listed above"),
        };
        let frame = controller.frame(viewport).unwrap();
        let flyout_rect = frame
            .rects()
            .flyout
            .filter(|flyout| flyout.panel == panel)
            .expect("open flyout should have current-frame geometry")
            .rect;
        let origin = Point::new(
            flyout_rect.origin.x + flyout_rect.size.width / 2.0,
            flyout_rect.origin.y + flyout_rect.size.height / 2.0,
        );
        let original_panel_count = controller.layout().panel_instances().count();

        let action = controller
            .commit_drop_request(
                DragDropRequest {
                    payload: DragPayload::Panel { panel },
                    handle: DragHandle::FlyoutPanel,
                    origin,
                    point: Point::new(420.0, 260.0),
                    target: Some(target),
                    floating_bounds: None,
                },
                viewport,
            )
            .unwrap();
        assert_eq!(
            action,
            DragDropAction::Command(DockCommand::MovePanelFromFlyout { panel, target })
        );
        assert!(controller.layout().flyout().is_none());
        assert_eq!(
            controller.layout().panel_instances().count(),
            original_panel_count
        );
        assert_ne!(
            group_containing_panel(&controller, panel),
            Some(source_group)
        );
        assert_eq!(controller.history.undo_len(), 1);
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        assert!(controller.undo().unwrap());
        assert!(controller.layout().flyout().is_none());
        let restored_group = group_containing_panel(&controller, panel)
            .expect("undo must restore the panel to a tab group");
        assert_ne!(
            restored_group, target_group,
            "undo must restore the panel outside the drop target; its source NodeId may be recreated"
        );
        assert_eq!(
            controller.layout().panel_instances().count(),
            original_panel_count
        );
        assert_eq!(controller.layout().floating_groups().count(), 0);
        assert_eq!(controller.history.undo_len(), 0);
        assert_eq!(controller.history.redo_len(), 1);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        assert!(controller.redo().unwrap());
        assert!(controller.layout().flyout().is_none());
        assert_eq!(
            controller.layout().panel_instances().count(),
            original_panel_count
        );
        assert_ne!(
            group_containing_panel(&controller, panel),
            Some(source_group)
        );
        assert_eq!(controller.history.undo_len(), 1);
        assert_eq!(controller.history.redo_len(), 0);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
    }
}

#[test]
fn singleton_panel_drop_on_own_group_edge_is_no_op_and_preserves_redo() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (group, _, panel) = controller
        .layout()
        .nodes()
        .find_map(|(group, node)| {
            let Node::Tabs(tabs) = node else {
                return None;
            };
            if tabs.children.len() != 1 {
                return None;
            }
            let Node::Panel(panel) = controller.layout().node(tabs.children[0])? else {
                return None;
            };
            Some((group, tabs.children[0], panel.panel_id))
        })
        .expect("initial layout should contain a single-tab group");

    controller
        .apply(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::DockHidden,
        })
        .unwrap();
    assert!(controller.undo().unwrap());
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);

    let before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();
    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                DropTarget::GroupEdge {
                    group,
                    zone: dock_core::DropZone::Right,
                },
                Point::new(180.0, 120.0),
                Point::new(420.0, 260.0),
            ),
            viewport,
        )
        .unwrap();

    assert_eq!(action, DragDropAction::NoOp);
    assert_eq!(layout_debug_snapshot(&controller), before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert!(controller.redo().unwrap());
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn panel_reorder_commits_once_and_own_group_edge_drop_is_no_op() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (group, original_order, moving_node, panel) = controller
        .layout()
        .nodes()
        .find_map(|(group, node)| {
            let Node::Tabs(tabs) = node else {
                return None;
            };
            if tabs.children.len() < 2 {
                return None;
            }
            let original_order = tabs.children.clone();
            let moving_node = *original_order.last()?;
            let Node::Panel(panel) = controller.layout().node(moving_node)? else {
                return None;
            };
            Some((group, original_order, moving_node, panel.panel_id))
        })
        .expect("initial layout should contain a tab group with at least two panels");
    let mut reordered = original_order.clone();
    let last_node = reordered
        .pop()
        .expect("the selected group has at least two tabs");
    assert_eq!(last_node, moving_node);
    reordered.insert(0, moving_node);
    let reorder_target = DropTarget::Tabs { group, index: 0 };

    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                reorder_target,
                Point::new(180.0, 120.0),
                Point::new(240.0, 180.0),
            ),
            viewport,
        )
        .unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MovePanel {
            panel,
            target: reorder_target,
        })
    );
    match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => assert_eq!(tabs.children, reordered),
        other => panic!("source group should remain a Tabs node, got {other:?}"),
    }
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => assert_eq!(tabs.children, original_order),
        other => panic!("undo should restore the original Tabs node, got {other:?}"),
    }
    assert!(controller.redo().unwrap());
    match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => assert_eq!(tabs.children, reordered),
        other => panic!("redo should restore the reordered Tabs node, got {other:?}"),
    }
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    // A panel dropped on its own group's edge is a no-op, even when
    // the group contains multiple tabs; detaching requires a different target group.
    controller.history.clear();
    let split_target = DropTarget::GroupEdge {
        group,
        zone: dock_core::DropZone::Right,
    };
    let before_edge_drop = layout_debug_snapshot(&controller);
    let version_before_edge_drop = controller.layout().version();
    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                split_target,
                Point::new(180.0, 120.0),
                Point::new(420.0, 260.0),
            ),
            viewport,
        )
        .unwrap();

    assert_eq!(action, DragDropAction::NoOp);
    assert_eq!(layout_debug_snapshot(&controller), before_edge_drop);
    assert_eq!(controller.layout().version(), version_before_edge_drop);
    assert_eq!(group_containing_panel(&controller, panel), Some(group));
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn incompatible_drag_targets_are_rejected_without_mutating_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let samples = first_panels_by_group(&controller);
    let (source_group, _, panel) = samples[0];
    let target_group = samples
        .iter()
        .map(|(group, _, _)| *group)
        .find(|group| *group != source_group)
        .expect("initial layout should contain a distinct target group");
    let initial_layout = layout_debug_snapshot(&controller);
    let initial_version = controller.layout().version();

    let panel_to_group_tabs = controller.commit_drop_request(
        drop_request(
            panel,
            DropTarget::GroupTabs {
                group: target_group,
                index: 0,
            },
            Point::new(180.0, 120.0),
            Point::new(420.0, 260.0),
        ),
        viewport,
    );
    assert!(panel_to_group_tabs.is_err());
    assert_eq!(layout_debug_snapshot(&controller), initial_layout);
    assert_eq!(controller.layout().version(), initial_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    let group_to_tabs = controller.commit_drop_request(
        DragDropRequest {
            payload: DragPayload::Group {
                group: source_group,
            },
            handle: DragHandle::GroupGrip,
            origin: Point::new(180.0, 120.0),
            point: Point::new(420.0, 260.0),
            target: Some(DropTarget::Tabs {
                group: target_group,
                index: 0,
            }),
            floating_bounds: None,
        },
        viewport,
    );
    assert!(group_to_tabs.is_err());
    assert_eq!(layout_debug_snapshot(&controller), initial_layout);
    assert_eq!(controller.layout().version(), initial_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn free_area_panel_drop_computes_bounds_floats_panel_and_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (source_group, panel_node, panel) = first_panels_by_group(&controller)[0];
    let frame = controller.frame(viewport).unwrap();
    let source_rect = frame
        .rects()
        .groups
        .iter()
        .find(|entry| entry.group == source_group)
        .expect("source group should have computed geometry")
        .rect;
    let origin = Point::new(
        source_rect.origin.x + 40.0,
        source_rect.origin.y + controller.metrics().header_height + 12.0,
    );
    let pointer = Point::new(600.0, 400.0);
    let original_version = controller.layout().version();
    let original_floating_count = controller.layout().floating_groups().count();

    let action = controller
        .commit_drop_request(
            free_area_request(
                DragPayload::Panel { panel },
                DragHandle::PanelTab,
                origin,
                pointer,
            ),
            viewport,
        )
        .unwrap();

    let bounds = match action {
        DragDropAction::Command(DockCommand::Float { node, bounds }) => {
            assert_eq!(node, panel_node, "panel detach must float the panel node");
            bounds
        }
        other => panic!("expected a Float command for the panel, got {other:?}"),
    };
    assert_bounds_inside(bounds, viewport);
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count + 1
    );
    assert!(controller.layout().version() > original_version);
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn free_area_group_drop_floats_the_complete_group_and_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (group, _, _) = first_panels_by_group(&controller)[0];
    let frame = controller.frame(viewport).unwrap();
    let source_rect = frame
        .rects()
        .groups
        .iter()
        .find(|entry| entry.group == group)
        .expect("source group should have computed geometry")
        .rect;
    let origin = Point::new(
        source_rect.origin.x + 35.0,
        source_rect.origin.y + controller.metrics().header_height / 2.0,
    );
    let pointer = Point::new(620.0, 390.0);
    let original_version = controller.layout().version();
    let original_floating_count = controller.layout().floating_groups().count();

    let action = controller
        .commit_drop_request(
            free_area_request(
                DragPayload::Group { group },
                DragHandle::GroupGrip,
                origin,
                pointer,
            ),
            viewport,
        )
        .unwrap();

    let bounds = match action {
        DragDropAction::Command(DockCommand::Float { node, bounds }) => {
            assert_eq!(
                node, group,
                "group-grip detach must float the complete Tabs node"
            );
            bounds
        }
        other => panic!("expected a Float command for the group, got {other:?}"),
    };
    assert_bounds_inside(bounds, viewport);
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count + 1
    );
    assert!(controller.layout().version() > original_version);
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert!(controller.redo().unwrap());
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count + 1
    );
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn flyout_panel_drop_to_dock_uses_move_panel_from_flyout_and_records_one_undo_step() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (source_column, _source_group, panel) = controller
        .layout()
        .nodes()
        .find_map(|(column_id, node)| {
            let Node::Column(column) = node else {
                return None;
            };
            column.groups.iter().find_map(|group_id| {
                let Node::Tabs(tabs) = controller.layout().node(*group_id)? else {
                    return None;
                };
                let panel_node = *tabs.children.get(tabs.active_index)?;
                let Node::Panel(panel) = controller.layout().node(panel_node)? else {
                    return None;
                };
                Some((column_id, *group_id, panel.panel_id))
            })
        })
        .expect("an initial dock column should contain an active tab panel");

    controller
        .apply(DockCommand::SetColumnMode {
            column: source_column,
            mode: ColumnMode::IconsOnly,
        })
        .unwrap();
    // Keep only the drop under test in undo history.
    controller.history.clear();
    controller
        .apply(DockCommand::OpenFlyout {
            column: source_column,
            panel,
        })
        .unwrap();
    assert!(controller.layout().flyout().is_some());
    assert_eq!(controller.history.undo_len(), 0);

    let target_group = controller
        .layout()
        .nodes()
        .find_map(|(column_id, node)| {
            let Node::Column(column) = node else {
                return None;
            };
            if column_id == source_column {
                return None;
            }
            column
                .groups
                .iter()
                .copied()
                .find(|group| matches!(controller.layout().node(*group), Some(Node::Tabs(_))))
        })
        .expect("a different main-dock column should provide a docking target");
    let target_index = match controller.layout().node(target_group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => unreachable!("target_group was selected from a Tabs child"),
    };
    let frame = controller.frame(viewport).unwrap();
    let flyout_rect = frame
        .rects()
        .flyout
        .filter(|flyout| flyout.panel == panel)
        .expect("the current flyout should have geometry")
        .rect;
    let origin = Point::new(flyout_rect.origin.x + 24.0, flyout_rect.origin.y + 40.0);
    let target_rect = frame
        .rects()
        .groups
        .iter()
        .find(|group| group.group == target_group)
        .expect("target group should have geometry")
        .rect;
    let pointer = Point::new(
        target_rect.origin.x + target_rect.size.width / 2.0,
        target_rect.origin.y + target_rect.size.height / 2.0,
    );
    let original_panel_count = controller.layout().panel_instances().count();
    let original_floating_count = controller.layout().floating_groups().count();

    let action = controller
        .commit_drop_request(
            DragDropRequest {
                payload: DragPayload::Panel { panel },
                handle: DragHandle::FlyoutPanel,
                origin,
                point: pointer,
                target: Some(DropTarget::Tabs {
                    group: target_group,
                    index: target_index,
                }),
                floating_bounds: None,
            },
            viewport,
        )
        .unwrap();

    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MovePanelFromFlyout {
            panel,
            target: DropTarget::Tabs {
                group: target_group,
                index: target_index,
            },
        })
    );
    assert!(controller.layout().flyout().is_none());
    assert_eq!(
        group_containing_panel(&controller, panel),
        Some(target_group)
    );
    assert_eq!(
        controller.layout().panel_instances().count(),
        original_panel_count
    );
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert!(
        controller.layout().flyout().is_none(),
        "undo restores persistent panel placement but does not reopen transient flyout UI"
    );
    assert_ne!(
        group_containing_panel(&controller, panel),
        Some(target_group),
        "undo should return the panel to its original group"
    );
    assert_eq!(
        controller.layout().panel_instances().count(),
        original_panel_count
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.redo().unwrap());
    assert!(controller.layout().flyout().is_none());
    assert_eq!(
        group_containing_panel(&controller, panel),
        Some(target_group)
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn free_area_drop_from_flyout_uses_flyout_command_closes_flyout_and_undo_restores_panel() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (column_id, panel) = controller
        .layout()
        .nodes()
        .find_map(|(column_id, node)| {
            let Node::Column(column) = node else {
                return None;
            };
            column.groups.iter().find_map(|group_id| {
                let Node::Tabs(tabs) = controller.layout().node(*group_id)? else {
                    return None;
                };
                let panel_node = *tabs.children.get(tabs.active_index)?;
                let Node::Panel(panel) = controller.layout().node(panel_node)? else {
                    return None;
                };
                Some((column_id, panel.panel_id))
            })
        })
        .expect("an initial dock column should contain an active tab panel");

    controller
        .apply(DockCommand::SetColumnMode {
            column: column_id,
            mode: ColumnMode::IconsOnly,
        })
        .unwrap();

    // The rail mode is setup for this scenario, not part of the drop's undo step.
    controller.history.clear();
    controller
        .apply(DockCommand::OpenFlyout {
            column: column_id,
            panel,
        })
        .unwrap();
    assert!(controller.layout().flyout().is_some());
    assert_eq!(controller.history.undo_len(), 0);

    let frame = controller.frame(viewport).unwrap();
    let flyout_rect = frame
        .rects()
        .flyout
        .filter(|flyout| flyout.panel == panel)
        .expect("open flyout should have geometry")
        .rect;
    let origin = Point::new(flyout_rect.origin.x + 24.0, flyout_rect.origin.y + 40.0);
    let pointer = Point::new(650.0, 420.0);
    let original_floating_count = controller.layout().floating_groups().count();
    let original_panel_count = controller.layout().panel_instances().count();

    let action = controller
        .commit_drop_request(
            free_area_request(
                DragPayload::Panel { panel },
                DragHandle::FlyoutPanel,
                origin,
                pointer,
            ),
            viewport,
        )
        .unwrap();

    let bounds = match action {
        DragDropAction::Command(DockCommand::FloatPanelFromFlyout {
            panel: command_panel,
            bounds,
        }) => {
            assert_eq!(command_panel, panel);
            bounds
        }
        other => panic!("expected FloatPanelFromFlyout for a flyout detach, got {other:?}"),
    };
    assert_bounds_inside(bounds, viewport);
    assert!(controller.layout().flyout().is_none());
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count + 1
    );
    assert_eq!(
        controller.layout().panel_instances().count(),
        original_panel_count
    );
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.undo().unwrap());
    assert!(
        controller.layout().flyout().is_none(),
        "undo restores persistent placement, while transient flyout visibility stays closed"
    );
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count
    );
    assert_eq!(
        controller.layout().panel_instances().count(),
        original_panel_count
    );
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(controller.redo().unwrap());
    assert_eq!(
        controller.layout().floating_groups().count(),
        original_floating_count + 1
    );
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn release_outside_valid_drop_zones_finishes_drag_without_mutating_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (_, _, panel) = first_panels_by_group(&controller)[0];
    let frame = controller.frame(viewport).unwrap();
    let tab_rect = frame
        .rects()
        .tab_headers
        .iter()
        .find(|header| header.panel == panel)
        .map(|header| header.rect)
        .expect("initial panel should have a visible tab header");
    let origin = Point::new(
        tab_rect.origin.x + tab_rect.size.width / 2.0,
        tab_rect.origin.y + tab_rect.size.height / 2.0,
    );
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();

    let drag_threshold = controller.metrics().drag_threshold;
    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x + drag_threshold + 20.0, origin.y),
                viewport,
                Some(&frame),
            )
            .unwrap()
    );

    let release_outside = Point::new(-10_000.0, -10_000.0);
    let request = controller.finish_drag(release_outside, viewport).unwrap();
    assert!(
        request.is_none(),
        "an invalid release target must not emit a drop request"
    );
    assert!(controller.drag_overlay().is_none());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    // finish_drag must leave the controller idle, not stuck in a terminal gesture.
    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
    assert!(!controller.cancel_drag());
}

#[test]
fn cancelling_active_drag_clears_session_without_touching_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (_, _, panel) = first_panels_by_group(&controller)[0];
    let frame = controller.frame(viewport).unwrap();
    let tab_rect = frame
        .rects()
        .tab_headers
        .iter()
        .find(|header| header.panel == panel)
        .map(|header| header.rect)
        .expect("initial panel should have a visible tab header");
    let origin = Point::new(
        tab_rect.origin.x + tab_rect.size.width / 2.0,
        tab_rect.origin.y + tab_rect.size.height / 2.0,
    );
    let layout_before = layout_debug_snapshot(&controller);
    let version_before = controller.layout().version();

    let drag_threshold = controller.metrics().drag_threshold;
    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(
        controller
            .update_drag(
                Point::new(origin.x + drag_threshold + 20.0, origin.y),
                viewport,
                Some(&frame),
            )
            .unwrap()
    );

    assert!(controller.cancel_drag());
    assert!(controller.drag_overlay().is_none());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);

    // A fresh drag may begin immediately after cancellation.
    controller
        .begin_drag(DragPayload::Panel { panel }, DragHandle::PanelTab, origin)
        .unwrap();
    assert!(controller.cancel_drag());
    assert!(!controller.cancel_drag());
    assert_eq!(layout_debug_snapshot(&controller), layout_before);
    assert_eq!(controller.layout().version(), version_before);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
}

#[test]
fn no_op_drop_does_not_change_layout_or_history() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));

    // Keep a redo branch to prove that a no-op preserves existing history.
    controller
        .apply(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::DockHidden,
        })
        .unwrap();
    assert_eq!(controller.history.undo_len(), 1);
    assert!(controller.undo().unwrap());
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);

    let (group, _, panel) = first_panels_by_group(&controller)[0];
    let original_layout = layout_debug_snapshot(&controller);
    let original_version = controller.layout().version();

    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                DropTarget::Tabs { group, index: 0 },
                Point::new(100.0, 100.0),
                Point::new(100.0, 100.0),
            ),
            viewport,
        )
        .unwrap();

    assert_eq!(action, DragDropAction::NoOp);
    assert_eq!(layout_debug_snapshot(&controller), original_layout);
    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 1);
    assert!(!controller.undo().unwrap());
    assert_eq!(controller.history.redo_len(), 1);
    assert!(controller.redo().unwrap());
    assert_eq!(controller.history.undo_len(), 1);
    assert_eq!(controller.history.redo_len(), 0);
}

#[test]
fn rejected_drop_and_rejected_command_leave_layout_and_history_unchanged() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
    let (group, _, panel) = first_panels_by_group(&controller)[0];
    let original_layout = layout_debug_snapshot(&controller);
    let original_version = controller.layout().version();

    let invalid_request = drop_request(
        panel,
        DropTarget::Tabs { group, index: 0 },
        Point::new(f32::NAN, 100.0),
        Point::new(100.0, 100.0),
    );
    assert!(
        controller
            .commit_drop_request(invalid_request, viewport)
            .is_err()
    );
    assert_eq!(layout_debug_snapshot(&controller), original_layout);
    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert!(controller.last_error().is_some());

    // A command rejected by dock-core must also leave the live state and
    // history untouched; use a Tabs node where SetColumnMode requires Column.
    assert!(
        controller
            .apply(DockCommand::SetColumnMode {
                column: group,
                mode: ColumnMode::IconsOnly,
            })
            .is_err()
    );
    assert_eq!(layout_debug_snapshot(&controller), original_layout);
    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert!(controller.last_error().is_some());

    // A successful no-op clears the stale error without touching layout/history.
    let action = controller
        .commit_drop_request(
            drop_request(
                panel,
                DropTarget::Tabs { group, index: 0 },
                Point::new(100.0, 100.0),
                Point::new(100.0, 100.0),
            ),
            viewport,
        )
        .unwrap();
    assert_eq!(action, DragDropAction::NoOp);
    assert!(controller.last_error().is_none());
    assert_eq!(layout_debug_snapshot(&controller), original_layout);
    assert_eq!(controller.layout().version(), original_version);
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), 0);
}

#[test]
fn raising_floating_window_does_not_add_undo_or_clear_redo() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let groups: Vec<_> = controller
        .layout()
        .nodes()
        .filter_map(|(id, node)| matches!(node, Node::Tabs(_)).then_some(id))
        .take(2)
        .collect();
    assert_eq!(
        groups.len(),
        2,
        "the initial layout should contain two tab groups"
    );

    for (index, group) in groups.into_iter().enumerate() {
        controller
            .apply(DockCommand::Float {
                node: group,
                bounds: Rect::new(
                    Point::new(80.0 + index as f32 * 40.0, 60.0),
                    Size::new(400.0, 300.0),
                ),
            })
            .unwrap();
    }
    assert_eq!(controller.layout().floating_groups().count(), 2);
    assert_eq!(controller.history.undo_len(), 2);

    controller
        .apply(DockCommand::SetChrome {
            mode: dock_core::ChromeMode::DockHidden,
        })
        .unwrap();
    assert!(controller.undo().unwrap());
    assert_eq!(controller.history.undo_len(), 2);
    assert_eq!(controller.history.redo_len(), 1);

    let order_before: Vec<_> = controller
        .layout()
        .floating_groups_in_hit_test_order()
        .into_iter()
        .map(|(group, _)| group)
        .collect();
    assert_eq!(order_before.len(), 2);
    let group_to_raise = *order_before
        .last()
        .expect("there should be a backmost floating window");
    let version_before = controller.layout().version();

    controller
        .apply(DockCommand::RaiseFloating {
            group: group_to_raise,
        })
        .unwrap();

    let order_after: Vec<_> = controller
        .layout()
        .floating_groups_in_hit_test_order()
        .into_iter()
        .map(|(group, _)| group)
        .collect();
    assert_eq!(order_after.first(), Some(&group_to_raise));
    assert_eq!(controller.layout().version(), version_before + 1);
    assert_eq!(controller.history.undo_len(), 2);
    assert_eq!(controller.history.redo_len(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    assert!(
        controller.redo().unwrap(),
        "raising must preserve the redo branch"
    );
    assert_eq!(controller.history.redo_len(), 0);
}

#[test]
fn closing_floating_column_closes_each_group_and_keeps_each_close_undoable() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let (column_id, group_count) = controller
        .layout()
        .nodes()
        .find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => Some((id, column.groups.len())),
            _ => None,
        })
        .expect("initial layout should contain a multi-group column");
    let original_panel_count = controller.layout().panel_instances().count();
    controller
        .apply(DockCommand::Float {
            node: column_id,
            bounds: Rect::new(Point::new(60.0, 40.0), Size::new(560.0, 440.0)),
        })
        .unwrap();
    let (floating_id, root) = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, floating)| {
            let root = controller.layout().dock_area(floating.area).unwrap().root;
            (id, root)
        })
        .expect("floating a column should create a floating group");
    let Node::Column(column) = controller.layout().node(root).unwrap() else {
        panic!("floating column should remain a Column root");
    };
    assert_eq!(column.groups.len(), group_count);

    // Exclude setup from the close-history assertions.
    controller.history.clear();
    controller.close_floating(floating_id).unwrap();

    assert!(controller.layout().floating_groups().next().is_none());
    assert!(controller.layout().panel_instances().count() < original_panel_count);
    assert_eq!(controller.history.undo_len(), group_count);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    for _ in 0..group_count {
        assert!(controller.undo().unwrap());
    }
    assert_eq!(controller.history.undo_len(), 0);
    assert_eq!(controller.history.redo_len(), group_count);
    assert_eq!(controller.layout().floating_groups().count(), 1);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

    for _ in 0..group_count {
        assert!(controller.redo().unwrap());
    }
    assert_eq!(controller.layout().floating_groups().count(), 0);
    assert_eq!(controller.history.redo_len(), 0);
    assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
}

#[test]
fn rail_threshold_resize_is_one_undoable_action() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let viewport = Rect::new(Point::ZERO, Size::new(1600.0, 900.0));
    let frame = controller.frame(viewport).unwrap();
    let column = crate::tree_query::main_column_ids(controller.layout(), DockSide::Right)
        .first()
        .copied()
        .expect("initial right dock should contain a column");
    let rect = frame
        .rects()
        .columns
        .iter()
        .find(|entry| entry.column == column)
        .expect("outer column should have frame geometry")
        .rect;
    assert!(rect.size.width > 220.0);

    let original_width = match controller.layout().node(column) {
        Some(Node::Column(value)) => value.width,
        other => panic!("expected a Column, got {other:?}"),
    };
    let start = Point::new(rect.origin.x, rect.origin.y + rect.size.height / 2.0);
    let end = Point::new(start.x + rect.size.width - 219.0, start.y);
    let handle = ResizeHandle::OuterColumn {
        column,
        side: DockSide::Right,
    };

    controller.begin_resize(handle, start, &frame).unwrap();
    controller.update_resize(end).unwrap();
    assert!(matches!(
        controller.layout().node(column),
        Some(Node::Column(value)) if value.mode == ColumnMode::IconsOnly
    ));
    controller.commit_resize(end).unwrap();
    assert!(matches!(
        controller.layout().node(column),
        Some(Node::Column(value))
            if value.mode == ColumnMode::IconsOnly
                && (value.last_expanded_width.unwrap() - 219.0).abs() < 0.01
    ));

    assert!(controller.undo().unwrap());
    assert!(matches!(
        controller.layout().node(column),
        Some(Node::Column(value))
            if value.mode == ColumnMode::Expanded
                && (value.width - original_width).abs() < 0.01
    ));
    assert!(!controller.undo().unwrap());
    assert!(controller.redo().unwrap());
    assert!(matches!(
        controller.layout().node(column),
        Some(Node::Column(value))
            if value.mode == ColumnMode::IconsOnly
                && (value.last_expanded_width.unwrap() - 219.0).abs() < 0.01
    ));
}

#[test]
fn frame_prunes_scroll_handles_for_removed_columns() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let (column_id, _) = controller
        .layout()
        .nodes()
        .find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.len() > 1 => Some((id, column.groups.len())),
            _ => None,
        })
        .expect("initial layout should have a multi-group column");
    let stale_node = NodeId::null();

    controller
        .scroll_handles_mut()
        .insert(column_id, gpui::ScrollHandle::new());
    controller
        .scroll_handles_mut()
        .insert(stale_node, gpui::ScrollHandle::new());

    controller
        .apply(DockCommand::Float {
            node: column_id,
            bounds: Rect::new(Point::new(60.0, 40.0), Size::new(560.0, 440.0)),
        })
        .unwrap();
    let floating_id = controller
        .layout()
        .floating_groups()
        .next()
        .map(|(id, _)| id)
        .expect("floating the column should create a floating group");
    controller.close_floating(floating_id).unwrap();
    assert!(
        controller.layout().node(column_id).is_none(),
        "closing every group should remove the old column node"
    );

    controller
        .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
        .unwrap();

    assert!(
        !controller.scroll_handles().contains_key(&column_id),
        "a handle for a removed column must be discarded"
    );
    assert!(
        !controller.scroll_handles().contains_key(&stale_node),
        "handles without a live Column owner must be discarded"
    );
}

#[test]
fn frame_preserves_scroll_handles_for_existing_columns_without_overflow() {
    let mut controller = DockController::new_initial(&Theme::default()).unwrap();
    let column_id = controller
        .layout()
        .nodes()
        .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
        .expect("initial layout should have a column");
    controller
        .scroll_handles_mut()
        .insert(column_id, gpui::ScrollHandle::new());

    controller
        .frame(Rect::new(Point::ZERO, Size::new(1600.0, 1000.0)))
        .unwrap();

    assert!(
        controller.scroll_handles().contains_key(&column_id),
        "a live column should retain its handle even when it does not currently overflow"
    );
}

#[test]
fn initial_layout_rejects_invalid_rail_thresholds() {
    let mut theme = Theme::default();
    theme.metrics.rail_expand_width = theme.metrics.rail_collapse_width;

    assert!(
        DockController::new_initial(&theme).is_err(),
        "invalid rail hysteresis thresholds should be rejected, not silently disable rail transitions"
    );
}

#[test]
fn restored_layout_rejects_invalid_rail_thresholds() {
    let valid_theme = Theme::default();
    let controller = DockController::new_initial(&valid_theme).unwrap();
    let layout = controller.layout().clone();

    let mut invalid_theme = Theme::default();
    invalid_theme.metrics.rail_expand_width = invalid_theme.metrics.rail_collapse_width;

    assert!(
        DockController::new_with_layout(&invalid_theme, layout).is_err(),
        "restoring a layout with invalid theme metrics must fail explicitly"
    );
}

#[test]
fn initial_group_ratios_follow_theme_default_column_split_ratio() {
    let mut theme = Theme::default();
    let split_ratio = 0.63;
    theme.metrics.default_column_split_ratio = split_ratio;

    let controller = DockController::new_initial(&theme).unwrap();
    let ratios = controller
        .layout()
        .nodes()
        .find_map(|(_, node)| match node {
            Node::Column(column) if column.groups.len() == 2 => Some(column.group_ratios.clone()),
            _ => None,
        })
        .expect("initial layout should have a column with two tab groups");

    assert_eq!(ratios.len(), 2);
    assert!(
        (ratios[0] - split_ratio).abs() < 0.001,
        "upper group ratio should follow ThemeMetrics::default_column_split_ratio; got {:?}",
        ratios
    );
    assert!(
        (ratios[1] - (1.0 - split_ratio)).abs() < 0.001,
        "lower group ratio should use the remainder; got {:?}",
        ratios
    );
}
