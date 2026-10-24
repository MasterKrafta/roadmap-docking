    use super::DockController;
    use crate::dock::resize_session::StartGeometry;
    use crate::requests::ResizeHandle;
    use dock_core::{Axis, Node, Point, Rect, Size};
    use ui::Theme;

    #[test]
    fn initial_render_layout_is_valid() {
        let controller =
            DockController::new_initial(&Theme::default()).expect("initial layout should be valid");
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        assert_eq!(controller.layout().panel_instances().count(), 4);
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
