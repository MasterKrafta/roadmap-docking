    use super::DockController;
    use crate::dock::resize_session::StartGeometry;
    use crate::requests::ResizeHandle;
    use dock_core::{Axis, DockCommand, DockSide, Node, NodeId, Point, Rect, Size};
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
        assert!(!controller.undo().unwrap(), "one resize must create one undo step");
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
        assert_eq!(horizontal_split_widths(&controller, owner), original_widths);
        assert!(!controller.undo().unwrap(), "the resize gesture must have one undo step");
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
        assert!(!controller.undo().unwrap(), "one resize should create one undo step");
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
        assert!(!controller.undo().unwrap(), "a zero-delta resize must not add history");
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
        assert!(!controller.undo().unwrap(), "cancelled gesture must not add history");
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
        assert!(!controller.undo().unwrap(), "gesture should add exactly one undo step");
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
