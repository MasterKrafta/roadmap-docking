use super::*;

#[test]
fn c45_resize_groups_changes_only_adjacent_pair_and_reports_minimum_clamp() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let constraints = |min_height: f32, max_width: f32| {
        PanelConstraints::new(
            Size::new(120.0, min_height),
            Size::new(200.0, min_height.max(140.0)),
            Some(Size::new(max_width, 600.0)),
            false,
        )
    };
    let (_, panel_a) = builder.panel(
        PanelKey::new("c45-group-a").unwrap(),
        constraints(120.0, 500.0),
    );
    let (_, panel_b) = builder.panel(
        PanelKey::new("c45-group-b").unwrap(),
        constraints(100.0, 500.0),
    );
    let (_, panel_c) = builder.panel(
        PanelKey::new("c45-group-c").unwrap(),
        constraints(250.0, 500.0),
    );
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let group_c = builder.tabs(vec![panel_c], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group_a, group_b, group_c],
        vec![1.0 / 3.0, 0.25, 5.0 / 12.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let before_ratios = match layout.node(column).unwrap() {
        Node::Column(value) => value.group_ratios.clone(),
        _ => unreachable!(),
    };

    let outcome = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 500.0,
                available_height: 824.0,
            },
            &metrics,
        )
        .unwrap();

    assert!(matches!(
        outcome.actual_value,
        Some(CommandValue::GroupBoundaryDelta(delta)) if (delta - 50.0).abs() < 1.0e-3
    ));
    let Node::Column(value) = layout.node(column).unwrap() else {
        panic!("target should remain a Column");
    };
    let expected_weight_delta = 50.0 / 824.0;
    assert!((value.group_ratios[0] - (1.0 / 3.0 + expected_weight_delta)).abs() < 1.0e-4);
    assert!((value.group_ratios[1] - (0.25 - expected_weight_delta)).abs() < 1.0e-4);
    assert_eq!(value.group_ratios[2], before_ratios[2]);
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value)) if value.group_ratios == before_ratios
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));

    let min_clamp = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: -500.0,
                available_height: 824.0,
            },
            &metrics,
        )
        .unwrap();
    let expected_min_delta = 176.0 - 824.0 / 3.0;
    assert!(matches!(
        min_clamp.actual_value,
        Some(CommandValue::GroupBoundaryDelta(delta)) if (delta - expected_min_delta).abs() < 1.0e-3
    ));
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value)) if (value.group_ratios[0] - (176.0 / 824.0)).abs() < 1.0e-4
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_resize_groups_cannot_shrink_below_tab_chrome_and_panel_minimum() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let constraints = |min_height: f32| {
        PanelConstraints::new(
            Size::new(120.0, min_height),
            Size::new(200.0, min_height.max(140.0)),
            None,
            false,
        )
    };
    let (_, panel_a) = builder.panel(
        PanelKey::new("c45-chrome-minimum-a").unwrap(),
        constraints(120.0),
    );
    let (_, panel_b) = builder.panel(
        PanelKey::new("c45-chrome-minimum-b").unwrap(),
        constraints(100.0),
    );
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group_a, group_b],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    // available_height excludes the 4 px splitter. Both groups start at 176 px;
    // group A requires 28 px header + 28 px strip + 120 px panel content.
    let outcome = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: -1.0,
                available_height: 352.0,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::GroupBoundaryDelta(0.0))
    );
    assert_eq!(outcome.inverse, None);
    assert_eq!(outcome.new_version, 0);
    assert_eq!(format!("{layout:?}"), before);

    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1000.0, 356.0)),
            &metrics,
        )
        .unwrap();
    assert_eq!(
        rects
            .groups
            .iter()
            .find(|item| item.group == group_a)
            .unwrap()
            .rect
            .size
            .height,
        176.0
    );
    assert_eq!(
        rects
            .groups
            .iter()
            .find(|item| item.group == group_b)
            .unwrap()
            .rect
            .size
            .height,
        176.0
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn c45_resize_groups_uses_effective_collapsed_height_when_tab_strip_is_taller() {
    let metrics = DockMetrics {
        collapsed_group_height: 28.0,
        tab_strip_height: 42.0,
        ..DockMetrics::default()
    };
    let constraints = PanelConstraints::new(
        Size::new(120.0, 100.0),
        Size::new(200.0, 140.0),
        None,
        false,
    );
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, collapsed_panel) = builder.panel(
        PanelKey::new("c45-effective-collapsed-height").unwrap(),
        constraints,
    );
    let (_, expanded_panel) = builder.panel(
        PanelKey::new("c45-effective-expanded-height").unwrap(),
        constraints,
    );
    let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
    let expanded = builder.tabs(vec![expanded_panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![collapsed, expanded],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    // The effective collapsed height is 42 px, leaving only 169 px for an
    // expanded group whose minimum is 28 + 42 + 100 = 170 px.
    let error = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 5.0,
                available_height: 211.0,
            },
            &metrics,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        CommandError::InvalidValue {
            field: "available_height",
            ..
        }
    ));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);

    let outcome = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 5.0,
                available_height: 212.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::GroupBoundaryDelta(0.0))
    );
    assert_eq!(outcome.inverse, None);
    assert_eq!(outcome.new_version, 0);
    assert_eq!(layout.validate(&metrics), Ok(()));

    // Geometry consumes the same minimum contract: 42 px collapsed strip,
    // 170 px expanded group, and one 4 px splitter.
    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1000.0, 216.0)),
            &metrics,
        )
        .unwrap();
    assert_eq!(
        rects
            .groups
            .iter()
            .find(|item| item.group == collapsed)
            .unwrap()
            .rect
            .size
            .height,
        42.0
    );
    assert_eq!(
        rects
            .groups
            .iter()
            .find(|item| item.group == expanded)
            .unwrap()
            .rect
            .size
            .height,
        170.0
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn c45_collapsed_group_has_fixed_height_and_cannot_be_resized_at_its_boundary() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, collapsed_panel) =
        builder.panel(PanelKey::new("c45-collapsed").unwrap(), descriptors()[0].1);
    let (_, expanded_a_panel) =
        builder.panel(PanelKey::new("c45-expanded-a").unwrap(), descriptors()[0].1);
    let (_, expanded_b_panel) =
        builder.panel(PanelKey::new("c45-expanded-b").unwrap(), descriptors()[0].1);
    let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
    let expanded_a = builder.tabs(vec![expanded_a_panel], 0, false);
    let expanded_b = builder.tabs(vec![expanded_b_panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![collapsed, expanded_a, expanded_b],
        vec![0.5, 0.25, 0.25],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    let fixed_boundary = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 100.0,
                available_height: 500.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        fixed_boundary.actual_value,
        Some(CommandValue::GroupBoundaryDelta(0.0))
    );
    assert_eq!(fixed_boundary.inverse, None);
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);

    let too_short = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 10.0,
                available_height: 10.0,
            },
            &metrics,
        )
        .unwrap_err();
    assert!(matches!(
        too_short,
        CommandError::InvalidValue {
            field: "available_height",
            ..
        }
    ));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);

    let expanded_boundary = layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 1,
                delta: 10_000.0,
                available_height: 500.0,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        expanded_boundary.actual_value,
        Some(CommandValue::GroupBoundaryDelta(delta)) if (delta - 100.0).abs() < 1.0e-3
    ));
    let Node::Column(value) = layout.node(column).unwrap() else {
        panic!("target should remain a Column");
    };
    let expected_weight_delta = 100.0 * 0.5 / 472.0;
    assert_eq!(value.group_ratios[0], 0.5);
    assert!((value.group_ratios[1] - (0.25 + expected_weight_delta)).abs() < 1.0e-4);
    assert!((value.group_ratios[2] - (0.25 - expected_weight_delta)).abs() < 1.0e-4);
    assert!((value.group_ratios.iter().sum::<f32>() - 1.0).abs() < 1.0e-4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_reset_column_ratios_shares_free_height_only_between_expanded_groups() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..4 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("c45-reset-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        groups.push(builder.tabs(vec![panel], 0, index == 0 || index == 2));
    }
    let initial_ratios = vec![0.1, 0.2, 0.3, 0.4];
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        groups,
        initial_ratios.clone(),
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    let outcome = layout
        .apply(
            DockCommand::ResetColumnRatios {
                column,
                available_height: 500.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::GroupRatios(vec![0.0, 0.5, 0.0, 0.5]))
    );
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value)) if value.group_ratios == vec![0.0, 0.5, 0.0, 0.5]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value)) if value.group_ratios == initial_ratios
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_reset_group_ratios_respects_minimums_when_equal_share_would_be_too_small() {
    let metrics = DockMetrics {
        min_group_height: 80.0,
        ..DockMetrics::default()
    };
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, tall_panel) = builder.panel(
        PanelKey::new("c45-tall-group").unwrap(),
        PanelConstraints::new(
            Size::new(100.0, 160.0),
            Size::new(180.0, 200.0),
            None,
            false,
        ),
    );
    let (_, short_panel) = builder.panel(
        PanelKey::new("c45-short-group").unwrap(),
        descriptors()[0].1,
    );
    let tall_group = builder.tabs(vec![tall_panel], 0, false);
    let short_group = builder.tabs(vec![short_panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![tall_group, short_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    let outcome = layout
        .apply(
            DockCommand::ResetColumnRatios {
                column,
                available_height: 360.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::GroupRatios(vec![0.6, 0.4]))
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_resize_column_clamps_min_and_max_and_canvas_gets_remainder() {
    let metrics = DockMetrics::default();
    let constraints = PanelConstraints::new(
        Size::new(240.0, 100.0),
        Size::new(300.0, 180.0),
        Some(Size::new(500.0, 360.0)),
        false,
    );
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, panel) = builder.panel(PanelKey::new("c45-bounded-column").unwrap(), constraints);
    let group = builder.tabs(vec![panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    let min_outcome = layout
        .apply(
            DockCommand::ResizeColumn {
                column,
                width: 100.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        min_outcome.actual_value,
        Some(CommandValue::ColumnWidth(240.0))
    );
    assert!(matches!(layout.node(column), Some(Node::Column(value)) if value.width == 240.0));

    let max_outcome = layout
        .apply(
            DockCommand::ResizeColumn {
                column,
                width: 900.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        max_outcome.actual_value,
        Some(CommandValue::ColumnWidth(500.0))
    );
    assert!(matches!(layout.node(column), Some(Node::Column(value)) if value.width == 500.0));
    assert_eq!(layout.validate(&metrics), Ok(()));

    let mut canvas_builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, canvas_panel) = canvas_builder.panel(
        PanelKey::new("c45-canvas-column").unwrap(),
        descriptors()[0].1,
    );
    let canvas_group = canvas_builder.tabs(vec![canvas_panel], 0, false);
    let canvas_column = canvas_builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![canvas_group],
        vec![1.0],
    );
    canvas_builder.main_area(DockSide::Left, canvas_column);
    canvas_builder.canvas(CanvasState::new(Some(Rect::new(
        Point::new(metrics.default_column_width, 0.0),
        Size::new(800.0, 600.0),
    ))));
    let mut canvas_layout = canvas_builder.build();
    let canvas_outcome = canvas_layout
        .apply(
            DockCommand::ResizeColumn {
                column: canvas_column,
                width: 900.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        canvas_outcome.actual_value,
        Some(CommandValue::ColumnWidth(760.0))
    );
    let bounds = canvas_layout.canvas().last_known_bounds().unwrap();
    assert_eq!(bounds.origin.x, 760.0);
    assert_eq!(bounds.size.width, metrics.min_canvas_width);
    assert_eq!(canvas_layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_resize_column_rejects_interior_main_column_target() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut columns = Vec::new();
    for index in 0..2 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("c45-interior-{index}")).unwrap(),
            descriptors()[0].1,
        );
        let group = builder.tabs(vec![panel], 0, false);
        columns.push(builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        ));
    }
    let root = builder.split(Axis::X, columns.clone(), vec![0.5, 0.5]);
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    let error = layout
        .apply(
            DockCommand::ResizeColumn {
                column: columns[1],
                width: 340.0,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(error, CommandError::InvalidTarget { .. }));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_resize_column_boundary_preserves_pair_sum_and_clamps_min_max() {
    let metrics = DockMetrics::default();
    let left_constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(200.0, 160.0),
        Some(Size::new(300.0, 400.0)),
        false,
    );
    let right_constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(200.0, 160.0),
        Some(Size::new(500.0, 400.0)),
        false,
    );
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, left_panel) = builder.panel(
        PanelKey::new("c45-boundary-left").unwrap(),
        left_constraints,
    );
    let (_, right_panel) = builder.panel(
        PanelKey::new("c45-boundary-right").unwrap(),
        right_constraints,
    );
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let left_column = builder.column(
        280.0,
        crate::ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );
    let right_column = builder.column(
        280.0,
        crate::ColumnMode::Expanded,
        vec![right_group],
        vec![1.0],
    );
    let root = builder.split(Axis::X, vec![left_column, right_column], vec![0.5, 0.5]);
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();
    let pair_sum = 560.0;

    let max_clamp = layout
        .apply(
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: 10_000.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        max_clamp.actual_value,
        Some(CommandValue::ColumnBoundaryDelta(20.0))
    );
    let left_width = match layout.node(left_column).unwrap() {
        Node::Column(value) => value.width,
        _ => unreachable!(),
    };
    let right_width = match layout.node(right_column).unwrap() {
        Node::Column(value) => value.width,
        _ => unreachable!(),
    };
    assert_eq!(left_width, 300.0);
    assert_eq!(right_width, 260.0);
    assert!((left_width + right_width - pair_sum).abs() < 1.0e-4);

    let min_clamp = layout
        .apply(
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: -10_000.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        min_clamp.actual_value,
        Some(CommandValue::ColumnBoundaryDelta(-100.0))
    );
    let left_width = match layout.node(left_column).unwrap() {
        Node::Column(value) => value.width,
        _ => unreachable!(),
    };
    let right_width = match layout.node(right_column).unwrap() {
        Node::Column(value) => value.width,
        _ => unreachable!(),
    };
    assert_eq!(left_width, 200.0);
    assert_eq!(right_width, 360.0);
    assert!((left_width + right_width - pair_sum).abs() < 1.0e-4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_resize_rejects_non_finite_or_non_positive_inputs_atomically() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Photoshop);
    let column = layout.main_area(DockSide::Right).unwrap().root;
    let before = format!("{layout:?}");

    for width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
        let error = layout
            .apply(DockCommand::ResizeColumn { column, width }, &metrics)
            .unwrap_err();
        assert!(matches!(
            error,
            CommandError::InvalidValue { field: "width", .. }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
    }

    for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let error = layout
            .apply(
                DockCommand::ResizeGroups {
                    column,
                    boundary_index: 0,
                    delta,
                    available_height: 400.0,
                },
                &metrics,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CommandError::InvalidValue { field: "delta", .. }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
    }

    for available_height in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
        let error = layout
            .apply(
                DockCommand::ResizeGroups {
                    column,
                    boundary_index: 0,
                    delta: 1.0,
                    available_height,
                },
                &metrics,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CommandError::InvalidValue {
                field: "available_height",
                ..
            }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
    }
}

#[test]
fn c45_group_resize_rejects_insufficient_available_height_transactionally() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..2 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("c45-too-short-{index}")).unwrap(),
            PanelConstraints::new(
                Size::new(120.0, 100.0),
                Size::new(220.0, 180.0),
                None,
                false,
            ),
        );
        groups.push(builder.tabs(vec![panel], 0, false));
    }
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        groups,
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    let error = layout
        .apply(
            DockCommand::ResetColumnRatios {
                column,
                available_height: 150.0,
            },
            &metrics,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        CommandError::InvalidValue {
            field: "available_height",
            ..
        }
    ));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c45_column_boundary_and_ratio_reset_reject_non_finite_or_non_positive_inputs() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_a) = builder.panel(PanelKey::new("c45-invalid-a").unwrap(), descriptors()[0].1);
    let (_, panel_b) = builder.panel(PanelKey::new("c45-invalid-b").unwrap(), descriptors()[0].1);
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group_a, group_b],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let error = layout
            .apply(
                DockCommand::ResizeColumnBoundary {
                    side: DockSide::Right,
                    boundary_index: 0,
                    delta,
                },
                &metrics,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CommandError::InvalidValue { field: "delta", .. }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
    }

    for available_height in [f32::NAN, f32::INFINITY, 0.0, -10.0] {
        let error = layout
            .apply(
                DockCommand::ResetColumnRatios {
                    column,
                    available_height,
                },
                &metrics,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CommandError::InvalidValue {
                field: "available_height",
                ..
            }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
    }
    assert_eq!(layout.validate(&metrics), Ok(()));
}
