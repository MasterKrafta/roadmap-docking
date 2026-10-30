use super::{ComputeRectsError, minimum_node_size, weighted_sizes};
use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ColumnMode, DockMetrics, DockSide, GeometryError, LayoutRects, Node, PanelConstraints,
    PanelKey, Point, Profile, Rect, Size,
};

const PIXEL_TOLERANCE: f32 = 1.0e-3;

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

#[test]
fn weighted_sizes_assigns_f32_rounding_remainder_to_last_active_child() {
    let weights = [0.33333334_f32; 3];
    let available = 997.3_f32;
    let minimums = [1.0_f32; 3];

    let sizes = weighted_sizes(&weights, available, &minimums, Axis::Y)
        .expect("valid positive weights should allocate the full stack");

    let assigned_before_last = sizes[0] + sizes[1];
    assert_eq!(sizes[2], available - assigned_before_last);
    assert_ne!(
        sizes[2],
        available * weights[2] / weights.iter().sum::<f32>(),
        "the last child must absorb accumulated f32 rounding instead of taking its raw ratio"
    );
    assert!((sizes.iter().sum::<f32>() - available).abs() <= f32::EPSILON * available);
}

#[test]
fn compute_rects_preserves_metric_validation_reason() {
    let layout = crate::Layout::new(Profile::Generic);
    let metrics = DockMetrics {
        min_canvas_width: 0.0,
        ..DockMetrics::default()
    };

    assert_eq!(
        layout.compute_rects(viewport(1000.0, 500.0), &metrics),
        Err(ComputeRectsError::InvalidMetrics {
            reason: "min_canvas_width must be positive",
        })
    );
}

// Test-only stand-in for the renderer's final logical->physical pixel snap.
// Production dock-core geometry deliberately does not accept a scale factor.
fn snap_rect_at_render_boundary(rect: Rect, scale: f32) -> Rect {
    assert!(scale.is_finite() && scale > 0.0);
    let snap = |coordinate: f32| (coordinate * scale).round() / scale;
    let left = snap(rect.origin.x);
    let top = snap(rect.origin.y);
    let right = snap(rect.origin.x + rect.size.width);
    let bottom = snap(rect.origin.y + rect.size.height);
    Rect::new(Point::new(left, top), Size::new(right - left, bottom - top))
}

#[test]
fn columns_keep_model_width_and_canvas_receives_horizontal_remainder() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel_node) = builder.panel(key("rect-left"), constraints());
    let left_group = builder.tabs(vec![left_panel_node], 0, false);
    let left_column = builder.column(240.0, ColumnMode::Expanded, vec![left_group], vec![1.0]);

    let (_, inner_panel_node) = builder.panel(key("rect-inner"), constraints());
    let inner_group = builder.tabs(vec![inner_panel_node], 0, false);
    let inner_column = builder.column(300.0, ColumnMode::Expanded, vec![inner_group], vec![1.0]);
    let left_root = builder.split(Axis::X, vec![left_column, inner_column], vec![0.5, 0.5]);
    builder.main_area(DockSide::Left, left_root);

    let (_, right_panel_node) = builder.panel(key("rect-right"), constraints());
    let right_group = builder.tabs(vec![right_panel_node], 0, false);
    let right_column = builder.column(220.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
    builder.main_area(DockSide::Right, right_column);

    let layout = builder.build();
    let first = layout
        .compute_rects(viewport(1200.0, 700.0), &metrics)
        .unwrap();
    let second = layout
        .compute_rects(viewport(1300.0, 500.0), &metrics)
        .unwrap();

    assert_eq!(first.columns[0].rect.size.width, 240.0);
    assert_eq!(first.columns[1].rect.size.width, 300.0);
    assert_eq!(first.columns[2].rect.size.width, 220.0);
    assert_eq!(first.columns[0].rect.origin.x, 0.0);
    assert_eq!(first.columns[1].rect.origin.x, 244.0);
    assert_eq!(first.columns[2].rect.origin.x, 980.0);
    assert_eq!(first.canvas.unwrap().origin.x, 544.0);
    assert_eq!(first.canvas.unwrap().size.width, 436.0);
    assert_eq!(second.columns[0].rect.size.width, 240.0);
    assert_eq!(second.columns[1].rect.size.width, 300.0);
    assert_eq!(second.columns[2].rect.size.width, 220.0);
    assert_eq!(second.canvas.unwrap().size.width, 536.0);
    assert_eq!(first.canvas.unwrap().size.height, 700.0);
    assert_eq!(second.canvas.unwrap().size.height, 500.0);
    assert_eq!(first.splitters.len(), 1);
}

#[test]
fn collapsed_groups_keep_fixed_height_and_are_excluded_from_ratios() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for (index, collapsed) in [(0, true), (1, false), (2, false)] {
        let (_, panel_node) = builder.panel(key(&format!("ratio-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, collapsed));
    }
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        groups.clone(),
        vec![0.2, 0.3, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let heights: Vec<_> = rects
        .groups
        .iter()
        .map(|group| group.rect.size.height)
        .collect();
    assert_eq!(heights[0], metrics.collapsed_group_height);
    assert!((heights[1] / heights[2] - 0.3 / 0.5).abs() < PIXEL_TOLERANCE);
    assert!(
        (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
            < PIXEL_TOLERANCE
    );
    assert_eq!(rects.panels.len(), 2);
    assert!(
        rects
            .panels
            .iter()
            .all(|panel| panel.rect.size.height > 0.0)
    );
}

#[test]
fn ratio_allocation_clamps_small_group_to_its_panel_minimum_and_redistributes_remainder() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for (index, collapsed) in [(0, true), (1, false), (2, false)] {
        let (_, panel_node) = builder.panel(key(&format!("floor-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, collapsed));
    }
    let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.8, 0.19, 0.01]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let heights: Vec<_> = rects
        .groups
        .iter()
        .map(|group| group.rect.size.height)
        .collect();
    let minimum = metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;

    assert_eq!(heights[0], metrics.collapsed_group_height);
    assert!((heights[2] - minimum).abs() < PIXEL_TOLERANCE);
    assert!((heights[1] - 348.0).abs() < PIXEL_TOLERANCE);
    assert!(heights[2] >= minimum);
    assert!(
        (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
            < PIXEL_TOLERANCE
    );
}

#[test]
fn active_tab_gets_panel_bounds_and_all_tabs_get_header_rectangles() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (first_panel, first_node) = builder.panel(key("tab-first"), constraints());
    let (active_panel, active_node) = builder.panel(key("tab-active"), constraints());
    let group = builder.tabs(vec![first_node, active_node], 1, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();

    assert_eq!(rects.tab_headers.len(), 2);
    assert_eq!(rects.tab_headers[0].panel, first_panel);
    assert_eq!(rects.tab_headers[1].panel, active_panel);
    assert_eq!(rects.tab_strips[0].group, group);
    assert_eq!(rects.panels.len(), 1);
    assert_eq!(rects.panels[0].panel, active_panel);
    assert_eq!(
        rects.panels[0].rect.origin.y,
        metrics.header_height + metrics.tab_strip_height
    );
    assert!(rects.drop_zones.is_empty());
}

#[test]
fn floating_geometry_preserves_bounds_z_order_and_resize_handles() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(key("floating-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let model_bounds = Rect::new(Point::new(-30.0, 45.0), Size::new(360.0, 260.0));
    let floating = builder.floating_group(group, model_bounds, 9);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();

    let expected_bounds = Rect::new(Point::new(0.0, 45.0), Size::new(360.0, 260.0));
    assert_eq!(rects.floating_groups.len(), 1);
    assert_eq!(rects.floating_groups[0].group, floating);
    assert_eq!(rects.floating_groups[0].rect, expected_bounds);
    assert_eq!(rects.floating_groups[0].z_order, 9);
    assert_eq!(rects.floating_resize_edges.len(), 8);
    assert_eq!(rects.panels[0].panel, panel);
    assert_eq!(rects.panels[0].rect.origin.x, expected_bounds.origin.x);
    assert_eq!(
        rects.panels[0].rect.origin.y,
        expected_bounds.origin.y + metrics.header_height + metrics.tab_strip_height
    );
    // Viewport clamping applies to geometry output, not persistent model bounds.
    assert_eq!(
        layout.floating_group(floating).unwrap().bounds,
        model_bounds
    );
}

#[test]
fn single_group_floating_column_keeps_standalone_group_geometry() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_node) = builder.panel(key("floating-single-column"), constraints());
    let group = builder.tabs(vec![panel_node], 0, true);
    let column = builder.column(300.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    let bounds = Rect::new(Point::new(40.0, 50.0), Size::new(300.0, 260.0));
    builder.floating_group(column, bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();
    let group_rect = rects
        .groups
        .iter()
        .find(|entry| entry.group == group)
        .unwrap()
        .rect;

    assert_eq!(rects.floating_groups[0].rect, bounds);
    assert_eq!(group_rect.origin.y, bounds.origin.y);
    assert_eq!(
        group_rect.size.height,
        metrics.effective_collapsed_group_height()
    );
}

#[test]
fn floating_column_reserves_window_title_before_allocating_a_collapsed_first_group() {
    let metrics = DockMetrics {
        header_height: 32.0,
        tab_strip_height: 24.0,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Generic);

    let (_, collapsed_panel_node) = builder.panel(key("floating-column-collapsed"), constraints());
    let collapsed_group = builder.tabs(vec![collapsed_panel_node], 0, true);
    let (_, expanded_panel_node) = builder.panel(key("floating-column-expanded"), constraints());
    let expanded_group = builder.tabs(vec![expanded_panel_node], 0, false);
    let column = builder.column(
        300.0,
        ColumnMode::Expanded,
        vec![collapsed_group, expanded_group],
        vec![0.5, 0.5],
    );
    let bounds = Rect::new(Point::new(40.0, 50.0), Size::new(300.0, 400.0));
    builder.floating_group(column, bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();
    let first = rects
        .groups
        .iter()
        .find(|entry| entry.group == collapsed_group)
        .unwrap()
        .rect;
    let second = rects
        .groups
        .iter()
        .find(|entry| entry.group == expanded_group)
        .unwrap()
        .rect;
    let splitter = rects
        .splitters
        .iter()
        .find(|entry| entry.owner == column)
        .unwrap()
        .rect;

    assert_eq!(rects.floating_groups[0].rect, bounds);
    assert_eq!(first.origin.y, bounds.origin.y + metrics.header_height);
    assert_eq!(
        first.size.height,
        metrics.effective_collapsed_group_height()
    );
    assert!(
        second.origin.y >= first.origin.y + first.size.height + metrics.splitter_thickness,
        "the expanded sibling must begin after the collapsed row and splitter"
    );
    assert_eq!(
        splitter.origin.y + (metrics.splitter_hit_zone - metrics.splitter_thickness) / 2.0,
        first.origin.y + first.size.height
    );
    assert!(
        ((second.origin.y + second.size.height) - (bounds.origin.y + bounds.size.height)).abs()
            < PIXEL_TOLERANCE
    );
}

#[test]
fn floating_horizontal_split_reserves_title_for_each_collapsed_column_branch() {
    let metrics = DockMetrics {
        header_height: 32.0,
        tab_strip_height: 24.0,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Generic);

    let (_, collapsed_panel_node) = builder.panel(key("split-floating-collapsed"), constraints());
    let collapsed_group = builder.tabs(vec![collapsed_panel_node], 0, true);
    let left_column = builder.column(
        300.0,
        ColumnMode::Expanded,
        vec![collapsed_group],
        vec![1.0],
    );

    let (_, expanded_panel_node) = builder.panel(key("split-floating-expanded"), constraints());
    let expanded_group = builder.tabs(vec![expanded_panel_node], 0, false);
    let right_column = builder.column(300.0, ColumnMode::Expanded, vec![expanded_group], vec![1.0]);
    let root = builder.split(Axis::X, vec![left_column, right_column], vec![0.5, 0.5]);
    let bounds = Rect::new(Point::new(40.0, 50.0), Size::new(604.0, 400.0));
    builder.floating_group(root, bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();
    let collapsed = rects
        .groups
        .iter()
        .find(|entry| entry.group == collapsed_group)
        .unwrap()
        .rect;
    let expanded = rects
        .groups
        .iter()
        .find(|entry| entry.group == expanded_group)
        .unwrap()
        .rect;
    let collapsed_strip = rects
        .tab_strips
        .iter()
        .find(|entry| entry.group == collapsed_group)
        .unwrap()
        .rect;
    let expanded_strip = rects
        .tab_strips
        .iter()
        .find(|entry| entry.group == expanded_group)
        .unwrap()
        .rect;

    assert_eq!(rects.floating_groups[0].rect, bounds);
    assert_eq!(collapsed.origin.y, bounds.origin.y + metrics.header_height);
    assert_eq!(
        collapsed_strip.origin.y,
        bounds.origin.y + metrics.header_height
    );
    assert_eq!(expanded.origin.y, bounds.origin.y);
    assert_eq!(
        expanded_strip.origin.y,
        bounds.origin.y + metrics.header_height
    );
    assert!(
        (collapsed_strip.origin.y - expanded_strip.origin.y).abs() < PIXEL_TOLERANCE,
        "both horizontal branches must align their tab strips below the shared title"
    );
}

#[test]
fn floating_horizontal_split_minimum_includes_title_for_collapsed_branches() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut columns = Vec::new();
    let mut groups = Vec::new();

    for name in ["floating-min-split-left", "floating-min-split-right"] {
        let (_, panel_node) = builder.panel(key(name), constraints());
        let group = builder.tabs(vec![panel_node], 0, true);
        let column = builder.column(300.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        groups.push(group);
        columns.push(column);
    }

    let root = builder.split(Axis::X, columns, vec![0.5, 0.5]);
    let requested_bounds = Rect::new(Point::new(40.0, 50.0), Size::new(604.0, 40.0));
    let floating = builder.floating_group(root, requested_bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();
    let floating_rect = rects
        .floating_groups
        .iter()
        .find(|entry| entry.group == floating)
        .expect("floating split should have geometry")
        .rect;

    assert_eq!(
        floating_rect.size.height,
        metrics.header_height + metrics.effective_collapsed_group_height()
    );
    assert_eq!(
        layout.floating_group(floating).unwrap().bounds,
        requested_bounds,
        "geometry clamping must not mutate persistent model bounds"
    );
    for group in groups {
        let group_rect = rects
            .groups
            .iter()
            .find(|entry| entry.group == group)
            .expect("collapsed branch should have group geometry")
            .rect;
        assert_eq!(
            group_rect.origin.y,
            floating_rect.origin.y + metrics.header_height
        );
        assert_eq!(
            group_rect.size.height,
            metrics.effective_collapsed_group_height()
        );
    }
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn floating_multi_group_column_minimum_includes_title_for_collapsed_first_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, collapsed_panel) = builder.panel(key("floating-min-column-collapsed"), constraints());
    let collapsed_group = builder.tabs(vec![collapsed_panel], 0, true);
    let (_, expanded_panel) = builder.panel(key("floating-min-column-expanded"), constraints());
    let expanded_group = builder.tabs(vec![expanded_panel], 0, false);
    let column = builder.column(
        300.0,
        ColumnMode::Expanded,
        vec![collapsed_group, expanded_group],
        vec![0.5, 0.5],
    );
    let requested_bounds = Rect::new(Point::new(40.0, 50.0), Size::new(300.0, 100.0));
    let floating = builder.floating_group(column, requested_bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();
    let actual_bounds = rects
        .floating_groups
        .iter()
        .find(|entry| entry.group == floating)
        .expect("floating column should have geometry")
        .rect;

    let expanded_minimum = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height + constraints().min_size.height);
    let required_without_title =
        metrics.effective_collapsed_group_height() + expanded_minimum + metrics.splitter_thickness;
    assert_eq!(
        actual_bounds.size.height,
        required_without_title + metrics.header_height
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn floating_vertical_split_minimum_includes_title_for_collapsed_first_branch() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, collapsed_panel) = builder.panel(key("floating-y-min-collapsed"), constraints());
    let collapsed_group = builder.tabs(vec![collapsed_panel], 0, true);
    let (_, expanded_panel) = builder.panel(key("floating-y-min-expanded"), constraints());
    let expanded_group = builder.tabs(vec![expanded_panel], 0, false);
    let root = builder.split(
        Axis::Y,
        vec![collapsed_group, expanded_group],
        vec![0.5, 0.5],
    );
    let requested_bounds = Rect::new(Point::new(40.0, 50.0), Size::new(300.0, 100.0));
    let floating = builder.floating_group(root, requested_bounds, 1);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .expect("floating vertical split should reserve its title row");
    let actual_bounds = rects
        .floating_groups
        .iter()
        .find(|entry| entry.group == floating)
        .expect("floating vertical split should have geometry")
        .rect;
    let group_rect = |group| {
        rects
            .groups
            .iter()
            .find(|entry| entry.group == group)
            .expect("both split branches should have geometry")
            .rect
    };
    let collapsed_rect = group_rect(collapsed_group);
    let expanded_rect = group_rect(expanded_group);
    let expanded_minimum = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height + constraints().min_size.height);
    let required_without_title =
        metrics.effective_collapsed_group_height() + expanded_minimum + metrics.splitter_thickness;

    assert_eq!(
        actual_bounds.size.height,
        required_without_title + metrics.header_height
    );
    assert_eq!(
        collapsed_rect.origin.y,
        actual_bounds.origin.y + metrics.header_height
    );
    assert_eq!(
        collapsed_rect.size.height,
        metrics.effective_collapsed_group_height()
    );
    assert_eq!(
        expanded_rect.origin.y,
        collapsed_rect.origin.y + collapsed_rect.size.height + metrics.splitter_thickness
    );
    assert!(
        ((expanded_rect.origin.y + expanded_rect.size.height)
            - (actual_bounds.origin.y + actual_bounds.size.height))
            .abs()
            < PIXEL_TOLERANCE
    );
    assert_eq!(
        layout.floating_group(floating).unwrap().bounds,
        requested_bounds,
        "geometry clamping must not mutate persistent model bounds"
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn floating_collapsed_group_stays_strip_sized_inside_window_bounds() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(key("floating-collapsed"), constraints());
    let group = builder.tabs(vec![panel_node], 0, true);
    let bounds = Rect::new(Point::new(20.0, 30.0), Size::new(360.0, 260.0));
    builder.floating_group(group, bounds, 2);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 700.0), &metrics)
        .unwrap();

    assert_eq!(rects.floating_groups[0].rect, bounds);
    assert_eq!(rects.groups[0].rect.origin.y, bounds.origin.y);
    assert_eq!(rects.tab_strips[0].rect.origin.y, bounds.origin.y);
    assert_eq!(
        rects.groups[0].rect.size.height,
        metrics.collapsed_group_height
    );
    assert_eq!(
        rects.tab_strips[0].rect.size.height,
        metrics.collapsed_group_height
    );
    assert!(rects.panels.is_empty());
    assert!(layout.panel_instance(panel).is_some());
}

#[test]
fn icon_rail_uses_rail_width_and_emits_flattened_panel_targets() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(key("icon-rail-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();

    assert_eq!(rects.columns[0].rect.size.width, metrics.rail_width);
    assert_eq!(
        rects.groups[0].rect.size.height,
        metrics.collapsed_group_height
    );
    assert!(rects.tab_strips.is_empty());
    assert_eq!(rects.tab_headers.len(), 1);
    assert_eq!(rects.tab_headers[0].panel, panel);
    assert_eq!(rects.tab_headers[0].index, 0);
    assert_eq!(
        rects.tab_headers[0].rect.size.height,
        metrics.collapsed_group_height
    );
    assert!(rects.panels.is_empty());
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(column))
            if column.mode == ColumnMode::IconsOnly && column.width == metrics.rail_width
    ));
}

#[test]
fn golden_rects_for_single_left_column_are_exact_and_deterministic() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(key("golden-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let viewport = viewport(1000.0, 600.0);

    let rects = layout.compute_rects(viewport, &metrics).unwrap();
    assert_eq!(layout.compute_rects(viewport, &metrics).unwrap(), rects);

    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 600.0)))
    );
    assert_eq!(rects.columns.len(), 1);
    assert_eq!(
        rects.columns[0].rect,
        Rect::new(Point::ZERO, Size::new(280.0, 600.0))
    );
    assert_eq!(rects.groups.len(), 1);
    assert_eq!(
        rects.groups[0].rect,
        Rect::new(Point::ZERO, Size::new(280.0, 600.0))
    );
    assert_eq!(rects.tab_strips.len(), 1);
    assert_eq!(
        rects.tab_strips[0].rect,
        Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
    );
    assert_eq!(rects.tab_headers.len(), 1);
    assert_eq!(rects.tab_headers[0].panel, panel);
    assert_eq!(rects.tab_headers[0].index, 0);
    assert_eq!(
        rects.tab_headers[0].rect,
        Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
    );
    assert_eq!(rects.panels.len(), 1);
    assert_eq!(rects.panels[0].panel, panel);
    assert_eq!(
        rects.panels[0].rect,
        Rect::new(Point::new(0.0, 56.0), Size::new(280.0, 544.0))
    );
    assert!(rects.splitters.is_empty());
    assert!(rects.drop_zones.is_empty());
    assert!(rects.vertical_overflow.is_empty());
    assert!(layout.node(column).is_some());
    assert!(layout.node(group).is_some());
}

#[test]
fn golden_rects_for_1280_by_800_cover_both_docks_columns_groups_and_splitters() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, upper_panel_node) = builder.panel(key("golden-1280-left-upper"), constraints());
    let upper_group = builder.tabs(vec![upper_panel_node], 0, false);
    let (_, lower_panel_node) = builder.panel(key("golden-1280-left-lower"), constraints());
    let lower_group = builder.tabs(vec![lower_panel_node], 0, false);
    let first_column = builder.column(
        240.0,
        ColumnMode::Expanded,
        vec![upper_group, lower_group],
        vec![0.5, 0.5],
    );

    let (_, middle_panel_node) = builder.panel(key("golden-1280-left-middle"), constraints());
    let middle_group = builder.tabs(vec![middle_panel_node], 0, false);
    let second_column = builder.column(300.0, ColumnMode::Expanded, vec![middle_group], vec![1.0]);
    let left_root = builder.split(Axis::X, vec![first_column, second_column], vec![0.5, 0.5]);
    builder.main_area(DockSide::Left, left_root);

    let (_, right_panel_node) = builder.panel(key("golden-1280-right"), constraints());
    let right_group = builder.tabs(vec![right_panel_node], 0, false);
    let right_column = builder.column(220.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
    builder.main_area(DockSide::Right, right_column);

    let layout = builder.build();
    let rects = layout
        .compute_rects(viewport(1280.0, 800.0), &metrics)
        .unwrap();

    // Manual golden values: left dock = 240 + 4 + 300 = 544 px;
    // right dock = 220 px; Canvas gets the exact 516 px remainder.
    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::new(544.0, 0.0), Size::new(516.0, 800.0)))
    );
    assert_eq!(rects.columns.len(), 3);
    assert_eq!(
        rects
            .columns
            .iter()
            .map(|item| item.rect)
            .collect::<Vec<_>>(),
        vec![
            Rect::new(Point::ZERO, Size::new(240.0, 800.0)),
            Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
            Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
        ]
    );

    assert_eq!(rects.groups.len(), 4);
    assert_eq!(
        rects
            .groups
            .iter()
            .map(|item| item.rect)
            .collect::<Vec<_>>(),
        vec![
            Rect::new(Point::ZERO, Size::new(240.0, 398.0)),
            Rect::new(Point::new(0.0, 402.0), Size::new(240.0, 398.0)),
            Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
            Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
        ]
    );

    // Hit rectangles intentionally use the 8 px splitter hit zone,
    // centered around each 4 px visual splitter.
    assert_eq!(rects.splitters.len(), 2);
    assert_eq!(rects.splitters[0].owner, first_column);
    assert_eq!(rects.splitters[0].boundary_index, 0);
    assert_eq!(rects.splitters[0].axis, Axis::Y);
    assert_eq!(
        rects.splitters[0].rect,
        Rect::new(Point::new(0.0, 396.0), Size::new(240.0, 8.0))
    );
    assert_eq!(rects.splitters[1].owner, left_root);
    assert_eq!(rects.splitters[1].boundary_index, 0);
    assert_eq!(rects.splitters[1].axis, Axis::X);
    assert_eq!(
        rects.splitters[1].rect,
        Rect::new(Point::new(238.0, 0.0), Size::new(8.0, 800.0))
    );

    assert_eq!(rects.panels.len(), 4);
    assert_eq!(
        rects
            .panels
            .iter()
            .map(|item| item.rect)
            .collect::<Vec<_>>(),
        vec![
            Rect::new(Point::new(0.0, 56.0), Size::new(240.0, 342.0)),
            Rect::new(Point::new(0.0, 458.0), Size::new(240.0, 342.0)),
            Rect::new(Point::new(244.0, 56.0), Size::new(300.0, 744.0)),
            Rect::new(Point::new(1060.0, 56.0), Size::new(220.0, 744.0)),
        ]
    );
    assert!(rects.vertical_overflow.is_empty());
    assert!(rects.drop_zones.is_empty());
}

#[test]
fn fractional_logical_viewport_preserves_dpi_independent_geometry_and_reports_small_height_overflow()
 {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("fractional-logical-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(200.25, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    // Geometry remains in logical px: non-integral viewport origins, widths,
    // and heights are preserved exactly instead of rounded for a display DPI.
    let viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 196.5));
    let rects = layout.compute_rects(viewport, &metrics).unwrap();
    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::new(212.5, 7.5), Size::new(320.5, 196.5),))
    );
    assert_eq!(
        rects.columns[0].rect,
        Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 196.5),)
    );
    assert_eq!(
        rects.groups[0].rect,
        Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 196.5),)
    );
    assert_eq!(
        rects.panels[0].rect,
        Rect::new(Point::new(12.25, 63.5), Size::new(200.25, 140.5),)
    );
    assert!(rects.vertical_overflow.is_empty());

    // The panel's declared minimum height is 60 px, so the expanded group
    // minimum is header 28 + tab strip 28 + content 60 = 116 logical px.
    // A viewport 0.5 px below that minimum must report overflow.
    let short_viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 115.5));
    let short_rects = layout.compute_rects(short_viewport, &metrics).unwrap();
    assert_eq!(short_rects.vertical_overflow.len(), 1);
    assert_eq!(short_rects.vertical_overflow[0].owner, column);
    assert_eq!(
        short_rects.vertical_overflow[0].viewport,
        Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 115.5),)
    );
    assert_eq!(
        short_rects.vertical_overflow[0].content_extent,
        Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 116.0),)
    );
    assert_eq!(short_rects.groups[0].rect.size.height, 116.0);
    assert_eq!(short_rects.panels[0].rect.size.height, 60.0);

    // Crossing the minimum Canvas width by 0.25 logical px must fail,
    // rather than shrinking either dock or the Canvas.
    let too_narrow = Rect::new(Point::new(12.25, 7.5), Size::new(520.0, 196.5));
    assert!(matches!(
        layout.compute_rects(too_narrow, &metrics),
        Err(ComputeRectsError::InsufficientCanvasWidth { available, minimum })
            if available == 319.75 && minimum == metrics.min_canvas_width
    ));
}

#[test]
fn pixel_snapping_is_deferred_to_the_render_boundary_for_common_dpi_scales() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("dpi-boundary-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(200.25, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 196.5));
    let expected_logical_panel = Rect::new(Point::new(12.25, 63.5), Size::new(200.25, 140.5));
    let baseline = layout.compute_rects(viewport, &metrics).unwrap();
    assert_eq!(baseline.panels[0].rect, expected_logical_panel);

    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        // Recomputing core geometry must return identical logical-pixel data.
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(rects, baseline, "logical geometry changed at scale {scale}");
        let logical_panel = rects.panels[0].rect;
        assert_eq!(logical_panel, expected_logical_panel);

        // Snapping is a separate operation on a copy at the render boundary.
        let snapped = snap_rect_at_render_boundary(logical_panel, scale);
        assert_ne!(
            snapped, logical_panel,
            "fractional edges should snap at scale {scale}"
        );
        let physical_edges = [
            snapped.origin.x * scale,
            snapped.origin.y * scale,
            (snapped.origin.x + snapped.size.width) * scale,
            (snapped.origin.y + snapped.size.height) * scale,
        ];
        for edge in physical_edges {
            assert!(
                (edge - edge.round()).abs() < PIXEL_TOLERANCE,
                "edge {edge} is not pixel-aligned at scale {scale}"
            );
        }

        // Applying render snapping cannot mutate the geometry snapshot.
        assert_eq!(rects.panels[0].rect, expected_logical_panel);
    }
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn configurable_metrics_drive_header_strip_and_splitter_geometry() {
    let metrics = DockMetrics {
        header_height: 31.5,
        tab_strip_height: 20.25,
        splitter_thickness: 3.5,
        splitter_hit_zone: 9.0,
        ..DockMetrics::default()
    };
    assert_eq!(metrics.validate(), Ok(()));

    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for name in ["custom-metrics-upper", "custom-metrics-lower"] {
        let (_, panel_node) = builder.panel(key(name), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, false));
    }
    let column = builder.column(280.0, ColumnMode::Expanded, groups.clone(), vec![0.5, 0.5]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();

    assert_eq!(
        rects
            .tab_strips
            .iter()
            .map(|item| item.rect)
            .collect::<Vec<_>>(),
        vec![
            Rect::new(Point::new(0.0, 31.5), Size::new(280.0, 20.25)),
            Rect::new(Point::new(0.0, 283.25), Size::new(280.0, 20.25)),
        ]
    );
    assert_eq!(
        rects
            .panels
            .iter()
            .map(|item| item.rect)
            .collect::<Vec<_>>(),
        vec![
            Rect::new(Point::new(0.0, 51.75), Size::new(280.0, 196.5)),
            Rect::new(Point::new(0.0, 303.5), Size::new(280.0, 196.5)),
        ]
    );
    assert_eq!(rects.splitters.len(), 1);
    assert_eq!(rects.splitters[0].owner, column);
    assert_eq!(
        rects.splitters[0].rect,
        Rect::new(Point::new(0.0, 245.5), Size::new(280.0, 9.0))
    );
    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 500.0)))
    );

    // The model remains unchanged; these configurable presentation metrics
    // affect only the computed geometry snapshot.
    assert!(layout.node(column).is_some());
    assert_eq!(metrics.header_height, 31.5);
    assert_eq!(metrics.tab_strip_height, 20.25);
    assert_eq!(metrics.splitter_thickness, 3.5);
    assert_eq!(metrics.splitter_hit_zone, 9.0);
}

#[test]
fn floating_bounds_clamp_to_client_area_respect_minimums_and_sort_by_z() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut ids = Vec::new();
    for (name, z_order, origin, size) in [
        (
            "float-high",
            3,
            Point::new(-100.0, 650.0),
            Size::new(420.0, 360.0),
        ),
        (
            "float-low",
            1,
            Point::new(900.0, -50.0),
            Size::new(420.0, 360.0),
        ),
        (
            "float-mid",
            2,
            Point::new(30.0, 40.0),
            Size::new(100.0, 80.0),
        ),
    ] {
        let constraints = if name == "float-mid" {
            PanelConstraints::new(
                Size::new(160.0, 100.0),
                Size::new(180.0, 140.0),
                Some(Size::new(800.0, 600.0)),
                false,
            )
        } else {
            constraints()
        };
        let (_, panel_node) = builder.panel(key(name), constraints);
        let group = builder.tabs(vec![panel_node], 0, false);
        ids.push(builder.floating_group(group, Rect::new(origin, size), z_order));
    }
    let layout = builder.build();
    let client = Rect::new(Point::new(10.0, 20.0), Size::new(1000.0, 700.0));

    let rects = layout.compute_rects(client, &metrics).unwrap();

    assert_eq!(
        rects
            .floating_groups
            .iter()
            .map(|item| item.z_order)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(rects.floating_groups[0].group, ids[1]);
    assert_eq!(
        rects.floating_groups[0].rect,
        Rect::new(Point::new(590.0, 20.0), Size::new(420.0, 360.0))
    );
    assert_eq!(rects.floating_groups[1].group, ids[2]);
    assert_eq!(
        rects.floating_groups[1].rect,
        Rect::new(Point::new(30.0, 40.0), Size::new(160.0, 156.0))
    );
    assert_eq!(rects.floating_groups[2].group, ids[0]);
    assert_eq!(
        rects.floating_groups[2].rect,
        Rect::new(Point::new(10.0, 360.0), Size::new(420.0, 360.0))
    );
    let small_panel = layout.floating_group(ids[2]).unwrap().bounds;
    assert_eq!(small_panel.size, Size::new(100.0, 80.0));
    assert_eq!(
        layout.floating_group(ids[0]).unwrap().bounds.origin,
        Point::new(-100.0, 650.0)
    );
    assert_eq!(rects.floating_resize_edges.len(), 24);
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn vertical_split_reports_content_extent_when_child_minimums_do_not_fit() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut children = Vec::new();
    for name in ["split-top", "split-bottom"] {
        let (_, panel_node) = builder.panel(key(name), constraints());
        children.push(builder.tabs(vec![panel_node], 0, false));
    }
    let root = builder.split(Axis::Y, children, vec![0.5, 0.5]);
    builder.main_area(DockSide::Left, root);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 120.0), &metrics)
        .unwrap();

    assert_eq!(rects.vertical_overflow.len(), 1);
    assert_eq!(rects.vertical_overflow[0].owner, root);
    assert_eq!(rects.vertical_overflow[0].viewport.size.height, 120.0);
    assert_eq!(rects.vertical_overflow[0].content_extent.size.height, 236.0);
    assert_eq!(rects.panels.len(), 2);
    assert!(
        rects
            .panels
            .iter()
            .all(|panel| panel.rect.size.height == 60.0)
    );
}

#[test]
fn non_finite_viewport_is_rejected_without_emitting_non_finite_rectangles() {
    let layout = LayoutBuilder::new(Profile::Photoshop).build();
    let metrics = DockMetrics::default();
    for viewport in [
        Rect::new(Point::new(f32::NAN, 0.0), Size::new(1000.0, 700.0)),
        Rect::new(Point::ZERO, Size::new(f32::INFINITY, 700.0)),
        Rect::new(Point::ZERO, Size::new(1000.0, f32::INFINITY)),
    ] {
        assert!(matches!(
            layout.compute_rects(viewport, &metrics),
            Err(ComputeRectsError::Geometry(_))
        ));
    }
}

#[test]
fn unfit_floating_window_does_not_abort_geometry_for_other_areas() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (main_panel, main_panel_node) = builder.panel(key("float-fallback-main"), constraints());
    let main_group = builder.tabs(vec![main_panel_node], 0, false);
    let main_column = builder.column(280.0, ColumnMode::Expanded, vec![main_group], vec![1.0]);
    builder.main_area(DockSide::Left, main_column);

    let tight = PanelConstraints::new(
        Size::new(160.0, 100.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    );
    let (too_tall_panel, too_tall_panel_node) = builder.panel(key("float-too-tall"), tight);
    let too_tall_group = builder.tabs(vec![too_tall_panel_node], 0, false);
    let too_tall_floating = builder.floating_group(
        too_tall_group,
        Rect::new(Point::new(30.0, 20.0), Size::new(200.0, 90.0)),
        1,
    );

    let (visible_panel, visible_panel_node) =
        builder.panel(key("float-still-visible"), constraints());
    let visible_group = builder.tabs(vec![visible_panel_node], 0, false);
    let visible_floating = builder.floating_group(
        visible_group,
        Rect::new(Point::new(260.0, 20.0), Size::new(200.0, 90.0)),
        2,
    );
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 140.0), &metrics)
        .expect("one unfit floating window must not abort the whole snapshot");

    assert!(
        rects
            .columns
            .iter()
            .any(|column| column.column == main_column),
        "main dock geometry should still be available"
    );
    assert!(
        rects.panels.iter().any(|panel| panel.panel == main_panel),
        "main dock panel geometry should still be available"
    );
    assert!(
        rects
            .floating_groups
            .iter()
            .any(|group| group.group == visible_floating),
        "a valid floating window should remain visible"
    );
    assert!(
        !rects
            .floating_groups
            .iter()
            .any(|group| group.group == too_tall_floating),
        "a floating window that cannot fit should be omitted from this snapshot"
    );
    assert!(
        rects
            .panels
            .iter()
            .any(|panel| panel.panel == visible_panel),
        "the valid floating panel should remain available"
    );
    assert!(
        !rects
            .panels
            .iter()
            .any(|panel| panel.panel == too_tall_panel),
        "failed floating geometry must not leave partial panel rectangles"
    );
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn refuses_to_shrink_canvas_below_its_minimum_without_overflow_policy() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    for (side, key_text) in [
        (DockSide::Left, "canvas-left"),
        (DockSide::Right, "canvas-right"),
    ] {
        let (_, panel_node) = builder.panel(key(key_text), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(side, column);
    }
    let layout = builder.build();

    let result = layout.compute_rects(viewport(800.0, 500.0), &metrics);
    assert!(matches!(
        result,
        Err(ComputeRectsError::InsufficientCanvasWidth {
            available,
            minimum
        }) if available == 240.0 && minimum == metrics.min_canvas_width
    ));
}

#[test]
fn minimum_group_stack_reports_scroll_extent_instead_of_compressing_content() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..3 {
        let (_, panel_node) = builder.panel(key(&format!("short-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, index == 0));
    }
    let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.2, 0.3, 0.5]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 120.0), &metrics)
        .unwrap();
    let overflow = rects.vertical_overflow.as_slice();
    assert_eq!(overflow.len(), 1);
    assert_eq!(overflow[0].owner, column);
    assert_eq!(overflow[0].viewport, viewport(280.0, 120.0));
    assert_eq!(overflow[0].content_extent.origin, Point::ZERO);
    assert_eq!(overflow[0].content_extent.size, Size::new(280.0, 268.0));
    assert_eq!(rects.groups.len(), 3);
    assert_eq!(
        rects.groups[0].rect.size.height,
        metrics.collapsed_group_height
    );
    assert_eq!(rects.groups[1].rect.size.height, 116.0);
    assert_eq!(rects.groups[2].rect.size.height, 116.0);
    assert!(
        rects
            .panels
            .iter()
            .all(|panel| panel.rect.size.height >= constraints().min_size.height)
    );
}

fn assert_snapshot_rectangles_are_valid(rects: &LayoutRects) {
    crate::layout_rects::assert_layout_rects_invariants(rects);
}

#[test]
fn empty_column_minimum_size_is_invalid_layout_not_arithmetic_overflow() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let column = builder.column(280.0, ColumnMode::Expanded, Vec::new(), Vec::new());
    let layout = builder.build();

    assert!(matches!(
        minimum_node_size(
            &layout,
            column,
            &metrics,
            &mut std::collections::HashMap::new(),
        ),
        Err(ComputeRectsError::InvalidLayoutState)
    ));
}

#[test]
fn overflowing_minimum_size_still_reports_arithmetic_overflow() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let huge_constraints = PanelConstraints::new(
        Size::new(f32::MAX, 60.0),
        Size::new(f32::MAX, 140.0),
        Some(Size::new(f32::MAX, 600.0)),
        false,
    );
    let (_, first) = builder.panel(key("huge-first"), huge_constraints);
    let (_, second) = builder.panel(key("huge-second"), huge_constraints);
    let split = builder.split(Axis::X, vec![first, second], vec![0.5, 0.5]);
    let layout = builder.build();

    assert!(matches!(
        minimum_node_size(
            &layout,
            split,
            &metrics,
            &mut std::collections::HashMap::new(),
        ),
        Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow
        ))
    ));
}

#[test]
fn empty_layout_at_minimum_canvas_width_returns_full_valid_canvas() {
    let metrics = DockMetrics::default();
    let layout = LayoutBuilder::new(Profile::Photoshop).build();
    let window = Rect::new(
        Point::new(5.0, 9.0),
        Size::new(metrics.min_canvas_width, 1.0),
    );

    let rects = layout.compute_rects(window, &metrics).unwrap();

    assert_eq!(rects.canvas, Some(window));
    assert!(rects.columns.is_empty());
    assert!(rects.groups.is_empty());
    assert!(rects.tab_headers.is_empty());
    assert!(rects.tab_overflows.is_empty());
    assert!(rects.panels.is_empty());
    assert!(rects.floating_groups.is_empty());
    assert!(rects.vertical_overflow.is_empty());
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn narrow_window_fails_at_canvas_minimum_instead_of_emitting_negative_rectangles() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("narrow-canvas-boundary"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    // 280 px of dock plus the 319 px canvas leaves the Canvas one pixel
    // below its declared minimum of 320 logical px.
    assert!(matches!(
        layout.compute_rects(viewport(599.0, 120.0), &metrics),
        Err(ComputeRectsError::InsufficientCanvasWidth {
            available,
            minimum,
        }) if available == 319.0 && minimum == metrics.min_canvas_width
    ));
    // A window narrower than the dock must fail before constructing a
    // Canvas with negative width.
    assert!(matches!(
        layout.compute_rects(viewport(200.0, 120.0), &metrics),
        Err(ComputeRectsError::InsufficientCanvasWidth {
            available,
            minimum,
        }) if available == -80.0 && minimum == metrics.min_canvas_width
    ));

    let rects = layout
        .compute_rects(viewport(600.0, 120.0), &metrics)
        .unwrap();
    assert_eq!(
        rects.canvas,
        Some(Rect::new(Point::new(280.0, 0.0), Size::new(320.0, 120.0)))
    );
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn very_short_column_preserves_panel_minimum_and_reports_full_overflow_extent() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("short-window-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let short_window = viewport(1000.0, 1.0);

    let rects = layout.compute_rects(short_window, &metrics).unwrap();

    let minimum_group_height =
        metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;
    assert_eq!(rects.vertical_overflow.len(), 1);
    assert_eq!(rects.vertical_overflow[0].owner, column);
    assert_eq!(rects.vertical_overflow[0].viewport.size.height, 1.0);
    assert_eq!(
        rects.vertical_overflow[0].content_extent.size.height,
        minimum_group_height
    );
    assert_eq!(rects.groups[0].rect.size.height, minimum_group_height);
    assert_eq!(
        rects.panels[0].rect.size.height,
        constraints().min_size.height
    );
    assert!(rects.groups[0].rect.size.height >= metrics.min_group_height);
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn short_icon_rail_reports_full_scroll_extent_without_compressing_slots() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..3 {
        let (_, panel_node) = builder.panel(key(&format!("short-rail-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, false));
    }
    let column = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        groups.clone(),
        vec![1.0 / 3.0; 3],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    let rects = layout
        .compute_rects(viewport(1000.0, 20.0), &metrics)
        .unwrap();
    let slot_height = metrics.effective_collapsed_group_height();

    assert_eq!(rects.vertical_overflow.len(), 1);
    assert_eq!(rects.vertical_overflow[0].owner, column);
    assert_eq!(
        rects.vertical_overflow[0].content_extent.size.height,
        slot_height * 3.0
    );
    assert_eq!(rects.tab_headers.len(), 3);
    assert!(
        rects
            .tab_headers
            .iter()
            .all(|tab| tab.rect.size.height == slot_height)
    );
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn zero_sized_viewport_is_rejected_before_any_geometry_is_produced() {
    let metrics = DockMetrics::default();
    let layout = LayoutBuilder::new(Profile::Photoshop).build();

    for window in [
        Rect::new(Point::ZERO, Size::new(0.0, 100.0)),
        Rect::new(Point::ZERO, Size::new(100.0, 0.0)),
        Rect::new(Point::ZERO, Size::ZERO),
    ] {
        assert!(matches!(
            layout.compute_rects(window, &metrics),
            Err(ComputeRectsError::Geometry(_))
        ));
    }
}

#[test]
fn collapsed_group_unit_weight_does_not_leave_expanded_column_weight_sum_zero() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, expanded_panel) = builder.panel(key("zero-weight-expanded"), constraints());
    let (_, collapsed_panel) = builder.panel(key("unit-weight-collapsed"), constraints());
    let expanded = builder.tabs(vec![expanded_panel], 0, false);
    let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![expanded, collapsed],
        vec![0.0, 1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));

    // The collapsed group's unit ratio is a fixed strip, not a weight
    // available to the expanded group. Its zero ratio must trigger fallback.
    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .expect("valid ratios with a zero expanded-weight sum must lay out");
    let height = |group| {
        rects
            .groups
            .iter()
            .find(|item| item.group == group)
            .unwrap()
            .rect
            .size
            .height
    };
    assert!((height(expanded) - 468.0).abs() < PIXEL_TOLERANCE);
    assert_eq!(
        height(collapsed),
        metrics.effective_collapsed_group_height()
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn nested_vertical_split_shares_space_when_all_expanded_child_weights_are_zero() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_a) = builder.panel(key("nested-zero-a"), constraints());
    let (_, panel_b) = builder.panel(key("nested-zero-b"), constraints());
    let (_, collapsed_panel) = builder.panel(key("nested-zero-collapsed"), constraints());
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
    let vertical_split = builder.split(
        Axis::Y,
        vec![group_a, group_b, collapsed],
        vec![0.0, 0.0, 1.0],
    );
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![vertical_split],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));

    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .expect("nested Split(Y) must fall back when expanded weights sum to zero");
    let height = |group| {
        rects
            .groups
            .iter()
            .find(|item| item.group == group)
            .unwrap()
            .rect
            .size
            .height
    };
    assert!((height(group_a) - 232.0).abs() < PIXEL_TOLERANCE);
    assert!((height(group_b) - 232.0).abs() < PIXEL_TOLERANCE);
    assert_eq!(
        height(collapsed),
        metrics.effective_collapsed_group_height()
    );
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn zero_weight_child_keeps_its_minimum_without_changing_positive_weight_proportions() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_zero) = builder.panel(key("mixed-weights-zero"), constraints());
    let (_, panel_quarter) = builder.panel(key("mixed-weights-quarter"), constraints());
    let (_, panel_three_quarters) =
        builder.panel(key("mixed-weights-three-quarters"), constraints());
    let group_zero = builder.tabs(vec![panel_zero], 0, false);
    let group_quarter = builder.tabs(vec![panel_quarter], 0, false);
    let group_three_quarters = builder.tabs(vec![panel_three_quarters], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group_zero, group_quarter, group_three_quarters],
        vec![0.0, 0.25, 0.75],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    // ratio_issues must accept zero entries when the full vector still sums to one.
    assert_eq!(layout.validate(&metrics), Ok(()));

    let rects = layout
        .compute_rects(viewport(1000.0, 1000.0), &metrics)
        .expect("zero-weight children should be clamped without invalidating other weights");
    let height = |group| {
        rects
            .groups
            .iter()
            .find(|item| item.group == group)
            .unwrap()
            .rect
            .size
            .height
    };
    let zero_height = height(group_zero);
    let quarter_height = height(group_quarter);
    let three_quarters_height = height(group_three_quarters);

    let minimum = metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;
    assert!((zero_height - minimum).abs() < PIXEL_TOLERANCE);
    assert!((quarter_height - 219.0).abs() < PIXEL_TOLERANCE);
    assert!((three_quarters_height - 657.0).abs() < PIXEL_TOLERANCE);
    assert!((quarter_height / three_quarters_height - 0.25 / 0.75).abs() < PIXEL_TOLERANCE);
    assert!(rects.vertical_overflow.is_empty());
}

#[test]
fn dominant_ratio_keeps_zero_weight_groups_at_minimum_and_gives_remainder_to_dominant_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..3 {
        let (_, panel_node) = builder.panel(key(&format!("dominant-ratio-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, false));
    }
    let zero_a = groups[0];
    let zero_b = groups[1];
    let dominant = groups[2];
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        groups,
        vec![0.0, 0.0, 1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let viewport_height = 1000.0;
    let rects = layout
        .compute_rects(viewport(1000.0, viewport_height), &metrics)
        .expect("a dominant positive ratio should absorb the remaining height");

    let height = |wanted| {
        rects
            .groups
            .iter()
            .find(|group| group.group == wanted)
            .expect("every expanded group should emit geometry")
            .rect
            .size
            .height
    };
    let minimum = metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;

    assert!((height(zero_a) - minimum).abs() < PIXEL_TOLERANCE);
    assert!((height(zero_b) - minimum).abs() < PIXEL_TOLERANCE);
    assert!(
        (height(dominant) - (viewport_height - 2.0 * metrics.splitter_thickness - 2.0 * minimum))
            .abs()
            < PIXEL_TOLERANCE
    );
    assert!(height(dominant) > height(zero_a) * 2.0);
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn all_collapsed_groups_keep_their_fixed_strip_height_and_do_not_overflow() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for index in 0..3 {
        let (_, panel_node) = builder.panel(key(&format!("all-collapsed-{index}")), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, true));
    }
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        groups.clone(),
        vec![0.0, 1.0, 0.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let rects = layout
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .expect("all-collapsed columns should be valid with a generous viewport");

    assert_eq!(rects.groups.len(), 3);
    assert!(rects.panels.is_empty());
    assert!(rects.vertical_overflow.is_empty());
    for group in groups {
        let rendered = rects
            .groups
            .iter()
            .find(|rendered| rendered.group == group)
            .expect("collapsed group should still emit its rail/header geometry");
        assert_eq!(
            rendered.rect.size.height,
            metrics.effective_collapsed_group_height()
        );
    }
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn ratio_allocation_remains_valid_through_multiple_nested_vertical_splits() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_a) = builder.panel(key("multi-level-a"), constraints());
    let (_, panel_b) = builder.panel(key("multi-level-b"), constraints());
    let (_, panel_c) = builder.panel(key("multi-level-c"), constraints());
    let (_, panel_d) = builder.panel(key("multi-level-d"), constraints());
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let group_c = builder.tabs(vec![panel_c], 0, false);
    let group_d = builder.tabs(vec![panel_d], 0, false);
    let inner = builder.split(Axis::Y, vec![group_b, group_c], vec![0.0, 1.0]);
    let outer = builder.split(
        Axis::Y,
        vec![group_a, inner, group_d],
        vec![0.25, 0.5, 0.25],
    );
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![outer],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let viewport_height = 1400.0;
    let rects = layout
        .compute_rects(viewport(1000.0, viewport_height), &metrics)
        .expect("nested valid ratios should allocate geometry through every split level");

    let height = |wanted| {
        rects
            .groups
            .iter()
            .find(|group| group.group == wanted)
            .expect("nested tab group should emit geometry")
            .rect
            .size
            .height
    };
    let minimum = metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;
    let outer_left = height(group_a);
    let outer_right = height(group_d);
    let inner_zero_weight = height(group_b);
    let inner_dominant = height(group_c);

    assert!((outer_left - outer_right).abs() < PIXEL_TOLERANCE);
    assert!((inner_zero_weight - minimum).abs() < PIXEL_TOLERANCE);
    assert!(inner_dominant > minimum);
    assert!(
        ((inner_zero_weight + inner_dominant + metrics.splitter_thickness) - 2.0 * outer_left)
            .abs()
            < PIXEL_TOLERANCE
    );
    assert!(rects.vertical_overflow.is_empty());
    assert_snapshot_rectangles_are_valid(&rects);
}

#[test]
fn compute_rects_rejects_child_rect_with_unrepresentable_edge() {
    let metrics = DockMetrics::default();
    let origin_x = 4_294_967_296.0;
    let viewport = Rect::new(Point::new(origin_x, 0.0), Size::new(4096.0, 800.0));
    assert_eq!(viewport.validate_non_empty(), Ok(()));

    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_a) = builder.panel(key("large-origin-a"), constraints());
    let (_, panel_b) = builder.panel(key("large-origin-b"), constraints());
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    // The first child gets the 200px minimum column width. At this
    // coordinate magnitude, f32 spacing is 512px, so that width rounds
    // back to the origin and must not produce a zero-width child.
    let split = builder.split(Axis::X, vec![group_a, group_b], vec![0.01, 0.99]);
    builder.main_area(DockSide::Left, split);
    let layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));
    assert!(matches!(
        layout.compute_rects(viewport, &metrics),
        Err(ComputeRectsError::Geometry(
            GeometryError::NonPositiveDimension
        ))
    ));
}

#[test]
fn floating_bounds_clamp_rejects_positive_but_unrepresentable_edge() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel) = builder.panel(key("large-origin-floating"), constraints());
    let root = builder.tabs(vec![panel], 0, false);
    let layout = builder.build();
    let bounds = Rect::new(Point::new(2_147_483_648.0, 0.0), Size::new(80.0, 300.0));

    assert_eq!(
        super::clamp_floating_bounds(&layout, root, bounds, viewport(800.0, 600.0), &metrics,),
        Err(ComputeRectsError::Geometry(
            GeometryError::NonPositiveDimension
        ))
    );
}

#[test]
fn floating_bounds_clamp_handles_rounding_at_large_client_origin() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_node) = builder.panel(
        key("large-origin-floating-clamp"),
        PanelConstraints::new(
            Size::new(160.0, 100.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        ),
    );
    let root = builder.tabs(vec![panel_node], 0, false);
    let layout = builder.build();

    // At 2^24, f32 spacing above the origin is two units. The client
    // width rounds its right edge down to +200, so deriving max_x as
    // checked_right - width can round below the origin.
    let origin_x = 16_777_216.0_f32;
    let client_area = Rect::new(Point::new(origin_x, 0.0), Size::new(200.6, 300.0));
    assert_eq!(client_area.validate_non_empty(), Ok(()));
    let bounds = Rect::new(Point::new(-50.0, 20.0), Size::new(300.0, 250.0));

    let clamped = super::clamp_floating_bounds(&layout, root, bounds, client_area, &metrics)
        .expect("rounding at a large origin must not be reported as arithmetic overflow");

    assert_eq!(clamped.origin.x, origin_x);
    assert_eq!(clamped.size.width, client_area.size.width);
    assert_eq!(clamped.checked_right(), client_area.checked_right());
    assert!(clamped.validate_non_empty().is_ok());
}

fn add_perf_tabs_group(
    builder: &mut LayoutBuilder,
    prefix: &str,
    panel_count: usize,
) -> crate::NodeId {
    let mut panels = Vec::with_capacity(panel_count);
    for panel_index in 0..panel_count {
        let panel_key = format!("{prefix}-p{panel_index}");
        let (_, panel) = builder.panel(key(&panel_key), constraints());
        panels.push(panel);
    }
    builder.tabs(panels, 0, false)
}

fn perf_wide_layout(metrics: &DockMetrics) -> (crate::Layout, Rect, &'static str) {
    let column_count = 10;
    let groups_per_column = 8;
    let panels_per_group = 3;
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut columns = Vec::with_capacity(column_count);

    for column_index in 0..column_count {
        let mut groups = Vec::with_capacity(groups_per_column);
        for group_index in 0..groups_per_column {
            groups.push(add_perf_tabs_group(
                &mut builder,
                &format!("rects-wide-c{column_index}-g{group_index}"),
                panels_per_group,
            ));
        }
        columns.push(builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            groups,
            vec![1.0 / groups_per_column as f32; groups_per_column],
        ));
    }

    let root = builder.split(
        Axis::X,
        columns,
        vec![1.0 / column_count as f32; column_count],
    );
    builder.main_area(DockSide::Left, root);
    let viewport_width = metrics.default_column_width * column_count as f32
        + metrics.splitter_thickness * (column_count - 1) as f32
        + metrics.min_canvas_width
        + 512.0;
    let viewport_height =
        groups_per_column as f32 * (metrics.min_group_height + metrics.splitter_thickness) + 800.0;
    (
        builder.build(),
        viewport(viewport_width, viewport_height),
        "wide-10x8x3",
    )
}

fn perf_deep_layout(metrics: &DockMetrics) -> (crate::Layout, Rect, &'static str) {
    let depth = 24;
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut root = add_perf_tabs_group(&mut builder, "rects-deep-leaf", 1);

    for level in 0..depth {
        let sibling = add_perf_tabs_group(&mut builder, &format!("rects-deep-level{level}"), 1);
        // Keep this fixture focused on recursion depth. The wide fixture
        // already exercises horizontal allocation; alternating X/Y splits
        // here can generate a minimum-width stress case that is unrelated
        // to repeated minimum-size traversal.
        root = builder.split(Axis::Y, vec![sibling, root], vec![0.5, 0.5]);
    }
    builder.main_area(DockSide::Left, root);

    let viewport_width = metrics.default_column_width + metrics.min_canvas_width + 800.0;
    let viewport_height =
        depth as f32 * (metrics.min_group_height + metrics.splitter_thickness) + 1200.0;
    (
        builder.build(),
        viewport(viewport_width, viewport_height),
        "deep-vertical-24",
    )
}

fn perf_many_tabs_layout(metrics: &DockMetrics) -> (crate::Layout, Rect, &'static str) {
    let tabs_count = 128;
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let group = add_perf_tabs_group(&mut builder, "rects-tabs", tabs_count);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);

    let viewport_width = metrics.default_column_width + metrics.min_canvas_width + 512.0;
    let viewport_height = metrics.min_group_height + 1200.0;
    (
        builder.build(),
        viewport(viewport_width, viewport_height),
        "tabs-128",
    )
}

fn visit_summary(
    visits: &std::collections::HashMap<crate::NodeId, super::visit_probe::NodeVisitCounts>,
    field: fn(&super::visit_probe::NodeVisitCounts) -> u64,
) -> (u64, usize, u64) {
    let total = visits.values().map(field).sum();
    let repeated_nodes = visits.values().filter(|counts| field(counts) > 1).count();
    let maximum = visits.values().map(field).max().unwrap_or(0);
    (total, repeated_nodes, maximum)
}

fn measure_compute_rects_case(
    layout: &crate::Layout,
    viewport: Rect,
    metrics: &DockMetrics,
    label: &str,
    iterations: usize,
) {
    use std::{hint::black_box, time::Instant};

    assert_eq!(layout.validate(metrics), Ok(()));
    let initial = layout
        .compute_rects(viewport, metrics)
        .expect("benchmark fixture must compute geometry");
    assert!(
        !initial.panels.is_empty(),
        "benchmark should lay out panels"
    );

    for _ in 0..iterations.min(8) {
        let rects = layout
            .compute_rects(viewport, metrics)
            .expect("benchmark fixture must compute geometry");
        black_box((rects.panels.len(), rects.groups.len(), rects.columns.len()));
    }

    let started = Instant::now();
    let mut panel_rects = 0;
    let mut group_rects = 0;
    for _ in 0..iterations {
        let rects = layout
            .compute_rects(viewport, metrics)
            .expect("benchmark fixture must compute geometry");
        panel_rects = rects.panels.len();
        group_rects = rects.groups.len();
        black_box((rects.panels.len(), rects.groups.len(), rects.columns.len()));
    }
    let elapsed = started.elapsed();
    assert!(panel_rects > 0);
    assert!(group_rects > 0);

    // A separate profiled call gathers per-node counts without timing the
    // enabled counter/locking overhead in the benchmark's elapsed duration.
    super::visit_probe::start();
    let profiled = layout.compute_rects(viewport, metrics);
    let visits = super::visit_probe::finish();
    let profiled = profiled.expect("profiled benchmark fixture must compute geometry");
    assert_eq!(profiled.panels.len(), panel_rects);
    assert_eq!(profiled.groups.len(), group_rects);

    let preferred = visit_summary(&visits, |counts| counts.preferred_width);
    let minimum_helper = visit_summary(&visits, |counts| counts.minimum_node_size_helper);
    let minimum_recursive = visit_summary(&visits, |counts| counts.recursive_minimum_node_size);
    let allocation_child = visit_summary(&visits, |counts| counts.allocate_stack_child);
    let total_node_visits: u64 = visits
        .values()
        .map(|counts| {
            counts.preferred_width
                + counts.minimum_node_size_helper
                + counts.recursive_minimum_node_size
                + counts.allocate_stack_child
        })
        .sum();
    let unique_nodes = visits.len();

    println!(
        "compute-rects baseline [{}]: nodes={}, panels={}, iterations={}, elapsed_ms={:.3}, us/call={:.3}, calls/s={:.1}, output_panels={}, output_groups={}, unique_instrumented_nodes={}, total_node_visits={}",
        label,
        layout.nodes().count(),
        layout.panel_instances().count(),
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64,
        iterations as f64 / elapsed.as_secs_f64(),
        panel_rects,
        group_rects,
        unique_nodes,
        total_node_visits,
    );
    println!(
        "compute-rects visits [{}]: preferred_width=total/{}/nodes_repeated/{}/max_per_node/{}, minimum_node_size_helper=total/{}/nodes_repeated/{}/max_per_node/{}, recursive_minimum_node_size=total/{}/nodes_repeated/{}/max_per_node/{}, allocate_stack_children=total/{}/nodes_repeated/{}/max_per_node/{}",
        label,
        preferred.0,
        preferred.1,
        preferred.2,
        minimum_helper.0,
        minimum_helper.1,
        minimum_helper.2,
        minimum_recursive.0,
        minimum_recursive.1,
        minimum_recursive.2,
        allocation_child.0,
        allocation_child.1,
        allocation_child.2,
    );
}

/// Manual release-mode baseline for node-size recomputation during geometry.
///
/// Run with one test thread so that per-node visit counters stay isolated:
/// cargo test -p dock-core --release benchmark_compute_rects_node_size_visits -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "manual performance baseline; run the documented release-mode command"]
fn benchmark_compute_rects_node_size_visits() {
    let metrics = DockMetrics::default();
    let cases = [
        perf_wide_layout(&metrics),
        perf_deep_layout(&metrics),
        perf_many_tabs_layout(&metrics),
    ];
    for (layout, viewport, label) in cases {
        let node_count = layout.nodes().count();
        let iterations = if node_count > 300 { 20 } else { 60 };
        measure_compute_rects_case(&layout, viewport, &metrics, label, iterations);
    }
}

#[test]
fn minimum_node_size_respects_metrics_between_geometry_passes() {
    let metrics = DockMetrics::default();
    let taller_metrics = DockMetrics {
        header_height: 70.0,
        tab_strip_height: 70.0,
        min_group_height: 140.0,
        ..metrics
    };
    assert_eq!(taller_metrics.validate(), Ok(()));

    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel) = builder.panel(key("minimum-size-metrics"), constraints());
    let group = builder.tabs(vec![panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let small_viewport = viewport(1000.0, 150.0);

    let default_rects = layout.compute_rects(small_viewport, &metrics).unwrap();
    let taller_rects = layout
        .compute_rects(small_viewport, &taller_metrics)
        .unwrap();
    let group_height = |rects: &LayoutRects| {
        rects
            .groups
            .iter()
            .find(|rendered| rendered.group == group)
            .unwrap()
            .rect
            .size
            .height
    };

    assert_eq!(group_height(&default_rects), 150.0);
    assert_eq!(group_height(&taller_rects), 200.0);
    assert!(default_rects.vertical_overflow.is_empty());
    assert_eq!(taller_rects.vertical_overflow.len(), 1);
}

#[test]
fn geometry_respects_column_modes() {
    let metrics = DockMetrics::default();
    let build_layout = |mode: ColumnMode| {
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel) = builder.panel(key("minimum-size-column-mode"), constraints());
        let group = builder.tabs(vec![panel], 0, false);
        let width = match mode {
            ColumnMode::Expanded => metrics.default_column_width,
            ColumnMode::IconsAndLabels => metrics.min_column_width,
            ColumnMode::IconsOnly => metrics.rail_width,
        };
        let column = builder.column(width, mode, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        (layout, column)
    };

    let (expanded, _) = build_layout(ColumnMode::Expanded);
    let (icons_and_labels, _) = build_layout(ColumnMode::IconsAndLabels);
    let (icons_only, _) = build_layout(ColumnMode::IconsOnly);

    let expanded_rects = expanded
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let labels_rects = icons_and_labels
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let icons_rects = icons_only
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();

    assert_eq!(
        expanded_rects.columns[0].rect.size.width,
        metrics.default_column_width
    );
    assert_eq!(
        labels_rects.columns[0].rect.size.width,
        metrics.min_column_width
    );
    assert_eq!(icons_rects.columns[0].rect.size.width, metrics.rail_width);
    assert_eq!(expanded_rects.panels.len(), 1);
    assert!(labels_rects.panels.is_empty());
    assert!(icons_rects.panels.is_empty());
}

#[test]
fn geometry_respects_collapsed_states() {
    let metrics = DockMetrics::default();
    let build_layout = |collapsed: bool| {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("minimum-size-collapsed"), constraints());
        let group = builder.tabs(vec![panel], 0, collapsed);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        (builder.build(), group)
    };

    let (expanded, expanded_group) = build_layout(false);
    let (collapsed, collapsed_group) = build_layout(true);
    let expanded_rects = expanded
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let collapsed_rects = collapsed
        .compute_rects(viewport(1000.0, 500.0), &metrics)
        .unwrap();
    let rendered_group_height = |rects: &LayoutRects, group| {
        rects
            .groups
            .iter()
            .find(|rendered| rendered.group == group)
            .unwrap()
            .rect
            .size
            .height
    };

    assert_eq!(
        rendered_group_height(&expanded_rects, expanded_group),
        500.0
    );
    assert_eq!(
        rendered_group_height(&collapsed_rects, collapsed_group),
        metrics.effective_collapsed_group_height()
    );
    assert_eq!(expanded_rects.panels.len(), 1);
    assert!(collapsed_rects.panels.is_empty());
}

#[test]
fn minimum_node_size_cache_reuses_subtree_values() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_node) = builder.panel(key("minimum-cache-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let mut cache = std::collections::HashMap::new();

    let first = layout
        .minimum_node_size_with_cache(column, &metrics, &mut cache)
        .expect("fixture should have a valid minimum size");
    assert_eq!(cache.len(), 3, "column, tab group, and panel are cached");

    let cached_count = cache.len();
    let second = layout
        .minimum_node_size_with_cache(column, &metrics, &mut cache)
        .expect("cached minimum size should remain available");

    assert_eq!(first, second);
    assert_eq!(
        cache.len(),
        cached_count,
        "a repeated query should reuse the cached subtree"
    );
}
