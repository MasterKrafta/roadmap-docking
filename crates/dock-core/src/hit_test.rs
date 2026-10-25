use std::collections::HashSet;

use crate::{
    ComputeRectsError, DockMetrics, DropTarget, DropZone, FloatingGroupRect, FlyoutRect, HitResult,
    Layout, LayoutRects, Node, NodeId, PanelId, Point, Rect, ResizeEdge, Size,
};

impl Layout {
    /// Hit-tests a logical-pixel point against a geometry snapshot previously
    /// produced for this same Layout.
    ///
    /// Floating windows form front-to-back layers ordered by descending
    /// z_order. Within a floating window resize grips beat its title band;
    /// then splitters, group headers, tab headers, drop zones and panel content
    /// are tested. Blank space inside the topmost floating window belongs to
    /// that window and does not leak hits to the content beneath it.
    ///
    /// Rectangles use half-open bounds. Invalid coordinates or malformed hit
    /// rectangles are reported instead of allowing NaN/Infinity to affect hit
    /// selection.
    pub fn hit_test(
        &self,
        point: Point,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<HitResult, ComputeRectsError> {
        metrics
            .validate()
            .map_err(|reason| ComputeRectsError::InvalidMetrics { reason })?;
        point.validate().map_err(ComputeRectsError::Geometry)?;

        let floating_order = floating_front_to_back(&rects.floating_groups);

        // Test front-to-back first. A hit inside a floating window only needs that
        // window's subtree; collecting the union for all windows can wait until the
        // point falls through to the main dock or Canvas.
        for floating in &floating_order {
            if !contains(floating.rect, point)? {
                continue;
            }

            // Resize zones are generated corner-first, so the exact corner hit
            // wins over the overlapping horizontal/vertical edge zones.
            for resize in &rects.floating_resize_edges {
                if resize.group == floating.group && contains(resize.rect, point)? {
                    return Ok(HitResult::FloatingResizeEdge {
                        group: floating.group,
                        edge: resize.edge,
                    });
                }
            }

            let title_height = metrics.header_height.min(floating.rect.size.height);
            let title = Rect::new(
                floating.rect.origin,
                crate::Size::new(floating.rect.size.width, title_height),
            );
            if contains(title, point)? {
                return Ok(HitResult::FloatingTitle {
                    group: floating.group,
                });
            }

            if let Some((nodes, panels)) = floating_subtree(self, floating.group)?
                && let Some(hit) =
                    hit_layout_geometry(self, point, rects, metrics, &nodes, &panels, true)?
            {
                return Ok(hit);
            }

            // A floating window is opaque for hit dispatch purposes: even blank
            // space in a front window must not activate a dock beneath it.
            return Ok(HitResult::FloatingGroup {
                group: floating.group,
            });
        }

        // The flyout overlays the main dock and Canvas, but floating windows
        // above it have already received first refusal in the loop above.
        if let Some(flyout) = rects.flyout {
            let resize_handle = flyout_resize_hit_rect(flyout, metrics)?;
            if contains(resize_handle, point)? {
                return Ok(HitResult::FlyoutResizeEdge {
                    column: flyout.column,
                    panel: flyout.panel,
                    edge: flyout.resize_edge,
                });
            }
            if contains(flyout.rect, point)? {
                return Ok(HitResult::FlyoutPanel {
                    column: flyout.column,
                    group: flyout.group,
                    panel: flyout.panel,
                });
            }
        }

        // Overflow content is larger than its visible viewport. Do not let
        // off-screen main-dock rectangles steal hits outside the client area.
        if let Some(canvas) = rects.canvas {
            let viewport_bottom = canvas
                .checked_bottom()
                .map_err(ComputeRectsError::Geometry)?;
            if point.y < canvas.origin.y || point.y >= viewport_bottom {
                return Ok(HitResult::Miss);
            }
        }

        // Main-dock hit filtering needs to exclude all floating subtrees. Do this
        // only after floating windows, flyout and viewport bounds have declined the point.
        let (floating_nodes, floating_panels) = all_floating_subtrees(self, &floating_order)?;
        if let Some(hit) = hit_layout_geometry(
            self,
            point,
            rects,
            metrics,
            &floating_nodes,
            &floating_panels,
            false,
        )? {
            return Ok(hit);
        }

        if let Some(canvas) = rects.canvas
            && contains(canvas, point)?
        {
            return Ok(HitResult::Canvas);
        }

        // Keep unknown points distinct from the central Canvas.
        Ok(HitResult::Miss)
    }
}

fn flyout_resize_hit_rect(
    flyout: FlyoutRect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let half_zone = metrics.splitter_hit_zone * 0.5;
    let edge_x = match flyout.resize_edge {
        ResizeEdge::Left => flyout.rect.origin.x,
        ResizeEdge::Right => flyout.rect.origin.x + flyout.rect.size.width,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let rect = Rect::new(
        Point::new(edge_x - half_zone, flyout.rect.origin.y),
        Size::new(metrics.splitter_hit_zone, flyout.rect.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(rect)
}

fn contains(rect: Rect, point: Point) -> Result<bool, ComputeRectsError> {
    rect.contains(point).map_err(ComputeRectsError::Geometry)
}

fn floating_front_to_back(floating: &[FloatingGroupRect]) -> Vec<FloatingGroupRect> {
    let mut ordered = floating.to_vec();
    ordered.sort_by_key(|item| std::cmp::Reverse(item.z_order));
    ordered
}

fn collect_subtree(
    layout: &Layout,
    root: NodeId,
    nodes: &mut HashSet<NodeId>,
    panels: &mut HashSet<PanelId>,
) -> Result<(), ComputeRectsError> {
    if !nodes.insert(root) {
        return Ok(());
    }
    match layout.node(root).ok_or(ComputeRectsError::InvalidLayoutState)? {
        Node::Split(split) => {
            for child in &split.children {
                collect_subtree(layout, *child, nodes, panels)?;
            }
        }
        Node::Column(column) => {
            for group in &column.groups {
                collect_subtree(layout, *group, nodes, panels)?;
            }
        }
        Node::Tabs(tabs) => {
            for child in &tabs.children {
                collect_subtree(layout, *child, nodes, panels)?;
            }
        }
        Node::Panel(panel) => {
            panels.insert(panel.panel_id);
        }
    }
    Ok(())
}

type FloatingSubtree = (HashSet<NodeId>, HashSet<PanelId>);

fn floating_subtree(
    layout: &Layout,
    id: crate::FloatingGroupId,
) -> Result<Option<FloatingSubtree>, ComputeRectsError> {
    let Some(floating) = layout.floating_group(id) else {
        return Ok(None);
    };
    let Some(area) = layout.dock_area(floating.area) else {
        return Ok(None);
    };
    let mut nodes = HashSet::new();
    let mut panels = HashSet::new();
    collect_subtree(layout, area.root, &mut nodes, &mut panels)?;
    Ok(Some((nodes, panels)))
}

fn all_floating_subtrees(
    layout: &Layout,
    floating: &[FloatingGroupRect],
) -> Result<(HashSet<NodeId>, HashSet<PanelId>), ComputeRectsError> {
    let mut nodes = HashSet::new();
    let mut panels = HashSet::new();
    for item in floating {
        let Some(group) = layout.floating_group(item.group) else {
            continue;
        };
        let Some(area) = layout.dock_area(group.area) else {
            continue;
        };

        // Collect directly into shared sets. Building a pair of fresh HashSets
        // per floating window and then extending the union repeats allocation
        // and hashing work on points outside every floating window.
        collect_subtree(layout, area.root, &mut nodes, &mut panels)?;
    }
    Ok((nodes, panels))
}

fn hit_layout_geometry(
    layout: &Layout,
    point: Point,
    rects: &LayoutRects,
    metrics: &DockMetrics,
    scope_nodes: &HashSet<NodeId>,
    scope_panels: &HashSet<PanelId>,
    include_only: bool,
) -> Result<Option<HitResult>, ComputeRectsError> {
    // Splitters win over the column/panel areas they overlap. Reversing the
    // deterministic geometry order gives nested/outer boundaries a stable tie.
    for splitter in rects.splitters.iter().rev() {
        if node_in_scope(splitter.owner, scope_nodes, include_only)
            && contains(splitter.rect, point)?
        {
            return Ok(Some(HitResult::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis: splitter.axis,
            }));
        }
    }

    // Overflow buttons are explicit interactive geometry and take priority
    // over ordinary tab/header fallback areas in their strip.
    for overflow in rects.tab_overflows.iter().rev() {
        if node_in_scope(overflow.group, scope_nodes, include_only)
            && contains(overflow.button_rect, point)?
        {
            return Ok(Some(HitResult::TabOverflowButton {
                group: overflow.group,
            }));
        }
    }

    // Drop zones exist only in a drag snapshot. They beat ordinary group/header
    // and tab hits, while splitters and explicit overflow buttons retain priority.
    // Use the same explicit priority as resolve_drop when zones overlap.
    let mut best_drop_zone = None;
    for (index, zone) in rects.drop_zones.iter().enumerate().rev() {
        if !target_in_scope(zone.target, scope_nodes, include_only) || !contains(zone.rect, point)?
        {
            continue;
        }
        let priority = drop_zone_priority(zone.zone);
        let replace = best_drop_zone
            .as_ref()
            .map(|(best_priority, _, _)| priority < *best_priority)
            .unwrap_or(true);
        if replace {
            best_drop_zone = Some((priority, index, zone));
        }
    }
    if let Some((_, _, zone)) = best_drop_zone {
        return Ok(Some(HitResult::DropZone {
            target: zone.target,
            zone: zone.zone,
        }));
    }

    // A GroupRect's upper header band is the non-floating group/column header.
    // Collapsed groups have no separate header: their tab strip starts at y=0.
    for group in rects.groups.iter().rev() {
        if !node_in_scope(group.group, scope_nodes, include_only) {
            continue;
        }
        if !rects
            .tab_strips
            .iter()
            .any(|strip| strip.group == group.group)
        {
            continue;
        }
        if matches!(layout.node(group.group), Some(Node::Tabs(tabs)) if tabs.collapsed) {
            continue;
        }
        let header_height = metrics.header_height.min(group.rect.size.height);
        let header = Rect::new(
            group.rect.origin,
            crate::Size::new(group.rect.size.width, header_height),
        );
        if contains(header, point)? {
            return Ok(Some(HitResult::Group { group: group.group }));
        }
    }

    for tab in rects.tab_headers.iter().rev() {
        if node_in_scope(tab.group, scope_nodes, include_only) && contains(tab.rect, point)? {
            return Ok(Some(HitResult::TabHeader {
                group: tab.group,
                panel: tab.panel,
                index: tab.index,
            }));
        }
    }

    for panel in rects.panels.iter().rev() {
        if panel_in_scope(panel.panel, scope_panels, include_only) && contains(panel.rect, point)? {
            return Ok(Some(HitResult::Panel { panel: panel.panel }));
        }
    }

    for strip in rects.tab_strips.iter().rev() {
        if node_in_scope(strip.group, scope_nodes, include_only) && contains(strip.rect, point)? {
            return Ok(Some(HitResult::TabStrip { group: strip.group }));
        }
    }

    for group in rects.groups.iter().rev() {
        if node_in_scope(group.group, scope_nodes, include_only) && contains(group.rect, point)? {
            return Ok(Some(HitResult::Group { group: group.group }));
        }
    }

    for column in rects.columns.iter().rev() {
        if node_in_scope(column.column, scope_nodes, include_only) && contains(column.rect, point)?
        {
            return Ok(Some(HitResult::Column {
                column: column.column,
            }));
        }
    }

    Ok(None)
}

fn node_in_scope(node: NodeId, scope_nodes: &HashSet<NodeId>, include_only: bool) -> bool {
    scope_nodes.contains(&node) == include_only
}

fn panel_in_scope(panel: PanelId, scope_panels: &HashSet<PanelId>, include_only: bool) -> bool {
    scope_panels.contains(&panel) == include_only
}

fn target_in_scope(target: DropTarget, scope_nodes: &HashSet<NodeId>, include_only: bool) -> bool {
    let target_node = match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::ColumnSlot { column, .. } | DropTarget::RailSlot { column, .. } => Some(column),
        DropTarget::MainColumn { .. } | DropTarget::FreeArea => {
            return !include_only;
        }
    };
    target_node.is_some_and(|node| node_in_scope(node, scope_nodes, include_only))
}

fn drop_zone_priority(zone: DropZone) -> u8 {
    match zone {
        DropZone::RailInsertion => 0,
        DropZone::TabInsertion => 1,
        DropZone::DockEdge => 2,
        DropZone::ColumnLeft | DropZone::ColumnRight => 3,
        DropZone::ColumnEnd => 4,
        DropZone::FloatingTop | DropZone::FloatingBottom => 5,
        DropZone::Top | DropZone::Bottom => 6,
        DropZone::Left | DropZone::Right => 7,
        DropZone::FloatingCenter => 8,
        DropZone::Center => 9,
        DropZone::FreeArea => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::{ComputeRectsError, HitResult, Point, Rect};
    use crate::layout_builder::LayoutBuilder;
    use crate::{
        Axis, ColumnMode, DockMetrics, DockSide, DropSource, DropTarget, DropZone,
        GeometryError, LayoutRects, PanelConstraints, PanelKey, Profile, ResizeEdge, Size,
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

    fn large_floating_fixture() -> (
        crate::Layout,
        crate::LayoutRects,
        crate::FloatingGroupId,
        crate::PanelId,
    ) {
        const WINDOW_COUNT: usize = 8;
        const COLUMNS_PER_WINDOW: usize = 2;
        const GROUPS_PER_COLUMN: usize = 3;
        const TABS_PER_GROUP: usize = 8;

        let metrics = DockMetrics::default();
        // This fixture deliberately uses a horizontal split of columns inside each
        // floating window. The Photoshop profile forbids that floating-root shape.
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut top_floating = None;
        let mut top_panel = None;

        for window_index in 0..WINDOW_COUNT {
            let mut columns = Vec::with_capacity(COLUMNS_PER_WINDOW);
            for column_index in 0..COLUMNS_PER_WINDOW {
                let mut groups = Vec::with_capacity(GROUPS_PER_COLUMN);
                for group_index in 0..GROUPS_PER_COLUMN {
                    let mut panel_nodes = Vec::with_capacity(TABS_PER_GROUP);
                    for tab_index in 0..TABS_PER_GROUP {
                        let panel_key = key(&format!(
                            "hit-bench-{window_index}-{column_index}-{group_index}-{tab_index}"
                        ));
                        let (panel_id, panel_node) = builder.panel(panel_key, constraints());
                        if window_index + 1 == WINDOW_COUNT
                            && column_index == 0
                            && group_index == 0
                            && tab_index == 0
                        {
                            top_panel = Some(panel_id);
                        }
                        panel_nodes.push(panel_node);
                    }
                    groups.push(builder.tabs(panel_nodes, 0, false));
                }
                columns.push(builder.column(
                    280.0,
                    ColumnMode::Expanded,
                    groups,
                    vec![1.0 / GROUPS_PER_COLUMN as f32; GROUPS_PER_COLUMN],
                ));
            }

            let root = builder.split(Axis::X, columns, vec![0.5; COLUMNS_PER_WINDOW]);
            let bounds = Rect::new(
                Point::new(
                    100.0 + window_index as f32 * 5.0,
                    100.0 + window_index as f32 * 3.0,
                ),
                Size::new(800.0, 700.0),
            );
            let floating = builder.floating_group(root, bounds, window_index as i32);
            if window_index + 1 == WINDOW_COUNT {
                top_floating = Some(floating);
            }
        }

        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1600.0, 1000.0), &metrics)
            .unwrap();
        (
            layout,
            rects,
            top_floating.expect("fixture must contain a frontmost floating window"),
            top_panel.expect("fixture must contain a frontmost panel"),
        )
    }

    #[test]
    fn hit_test_preserves_metric_validation_reason() {
        let layout = LayoutBuilder::new(Profile::Generic).build();
        let metrics = DockMetrics {
            min_canvas_width: 0.0,
            ..DockMetrics::default()
        };

        assert_eq!(
            layout.hit_test(Point::ZERO, &LayoutRects::default(), &metrics),
            Err(ComputeRectsError::InvalidMetrics {
                reason: "min_canvas_width must be positive",
            })
        );
    }

    #[test]
    fn blank_space_in_frontmost_floating_window_does_not_leak_through() {
        let metrics = DockMetrics::default();
        let (layout, rects, top_floating, _) = large_floating_fixture();
        // The floating root is a horizontal split of two 280px columns inside
        // an 800px window, leaving an intentional blank region on the right.
        let blank_point = Point::new(900.0, 250.0);
        let floating_rect = rects
            .floating_groups
            .iter()
            .find(|floating| floating.group == top_floating)
            .unwrap()
            .rect;
        assert!(floating_rect.contains(blank_point).unwrap());

        assert_eq!(
            layout.hit_test(blank_point, &rects, &metrics).unwrap(),
            HitResult::FloatingGroup { group: top_floating }
        );
    }

    /// Manual release-mode benchmark for hit-testing large floating subtrees.
    ///
    /// Run with one test thread:
    /// cargo test -p dock-core --release benchmark_hit_test_multiple_floating_subtrees -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore = "manual performance baseline; run the documented release-mode command"]
    fn benchmark_hit_test_multiple_floating_subtrees() {
        let metrics = DockMetrics::default();
        let (layout, rects, top_floating, top_panel) = large_floating_fixture();
        let node_count = layout.nodes().count();
        let active_panel_rects = rects.panels.len();
        let panel_instances = 8 * 2 * 3 * 8;
        let iterations = 500;

        let measure = |label: &str, point: Point, expected: HitResult| {
            let start = std::time::Instant::now();
            let mut actual = HitResult::Miss;
            for _ in 0..iterations {
                actual = std::hint::black_box(
                    layout.hit_test(point, &rects, &metrics).unwrap(),
                );
            }
            let elapsed = start.elapsed();
            assert_eq!(actual, expected);
            eprintln!(
                "hit-test benchmark [{label}]: floating_windows={}, nodes={node_count}, panel_instances={panel_instances}, active_panel_rects={active_panel_rects}, iterations={iterations}, elapsed_ms={:.3}, us/call={:.3}, final_hit={actual:?}",
                rects.floating_groups.len(),
                elapsed.as_secs_f64() * 1000.0,
                elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64,
            );
        };

        measure(
            "inside-front-floating",
            Point::new(250.0, 250.0),
            HitResult::Panel { panel: top_panel },
        );
        measure(
            "outside-floating",
            Point::new(1500.0, 900.0),
            HitResult::Canvas,
        );
        assert!(
            rects
                .floating_groups
                .iter()
                .any(|item| item.group == top_floating)
        );
    }

    #[test]
    fn every_floating_resize_zone_beats_title_drag() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("resize-zones"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let bounds = Rect::new(Point::new(100.0, 100.0), Size::new(300.0, 200.0));
        let floating = builder.floating_group(group, bounds, 1);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 700.0), &metrics)
            .unwrap();

        for (point, expected) in [
            (Point::new(250.0, 102.0), ResizeEdge::Top),
            (Point::new(397.0, 102.0), ResizeEdge::TopRight),
            (Point::new(398.0, 200.0), ResizeEdge::Right),
            (Point::new(397.0, 298.0), ResizeEdge::BottomRight),
            (Point::new(250.0, 298.0), ResizeEdge::Bottom),
            (Point::new(102.0, 298.0), ResizeEdge::BottomLeft),
            (Point::new(102.0, 200.0), ResizeEdge::Left),
            (Point::new(102.0, 102.0), ResizeEdge::TopLeft),
        ] {
            assert_eq!(
                layout.hit_test(point, &rects, &metrics).unwrap(),
                HitResult::FloatingResizeEdge {
                    group: floating,
                    edge: expected
                }
            );
        }

        assert_eq!(
            layout
                .hit_test(Point::new(140.0, 112.0), &rects, &metrics)
                .unwrap(),
            HitResult::FloatingTitle { group: floating }
        );
    }

    #[test]
    fn frontmost_floating_window_wins_overlap_and_zone_hits_do_not_leak() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (lower_panel, lower_node) = builder.panel(key("lower-window"), constraints());
        let lower_group = builder.tabs(vec![lower_node], 0, false);
        let bounds = Rect::new(Point::new(100.0, 80.0), Size::new(300.0, 220.0));
        let lower = builder.floating_group(lower_group, bounds, 1);

        let (upper_panel, upper_node) = builder.panel(key("upper-window"), constraints());
        let upper_group = builder.tabs(vec![upper_node], 0, false);
        let upper = builder.floating_group(upper_group, bounds, 2);

        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 700.0), &metrics)
            .unwrap();

        assert_eq!(
            rects
                .floating_groups
                .iter()
                .map(|group| group.z_order)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            layout
                .hit_test(Point::new(180.0, 170.0), &rects, &metrics)
                .unwrap(),
            HitResult::Panel { panel: upper_panel }
        );
        assert_ne!(lower_panel, upper_panel);

        // Drop zones are contextual: compute them for the dragged panel and
        // keep the front window's target from leaking through to the lower one.
        let drop_zones = layout
            .compute_drop_zones(DropSource::Panel { panel: lower_panel }, &rects, &metrics)
            .unwrap();
        let mut drag_rects = rects.clone();
        drag_rects.drop_zones = drop_zones;
        assert_eq!(
            layout
                .hit_test(Point::new(390.0, 280.0), &drag_rects, &metrics)
                .unwrap(),
            HitResult::DropZone {
                target: DropTarget::GroupEdge {
                    group: upper_group,
                    zone: crate::DropZone::Bottom
                },
                zone: crate::DropZone::FloatingBottom,
            }
        );
        assert_ne!(lower, upper);
    }

    #[test]
    fn drop_zone_precedence_makes_tab_insertions_and_group_edges_reachable() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, first_node) = builder.panel(key("drop-target-first"), constraints());
        let (_, second_node) = builder.panel(key("drop-target-second"), constraints());
        let target_group = builder.tabs(vec![first_node, second_node], 0, false);
        let target_column =
            builder.column(280.0, ColumnMode::Expanded, vec![target_group], vec![1.0]);
        builder.main_area(DockSide::Left, target_column);

        let (source_panel, source_node) = builder.panel(key("drop-source"), constraints());
        let source_group = builder.tabs(vec![source_node], 0, false);
        let source_column =
            builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
        builder.main_area(DockSide::Right, source_column);

        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1200.0, 800.0), &metrics)
            .unwrap();
        let zones = layout
            .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
            .unwrap();
        let mut drag_rects = rects.clone();
        drag_rects.drop_zones = zones.clone();

        let tab_zone = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::TabInsertion
                    && zone.target == (DropTarget::Tabs {
                        group: target_group,
                        index: 0,
                    })
            })
            .expect("the target tab should have an insertion zone");
        let tab_point = Point::new(
            tab_zone.rect.origin.x + tab_zone.rect.size.width / 2.0,
            tab_zone.rect.origin.y + tab_zone.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.hit_test(tab_point, &drag_rects, &metrics).unwrap(),
            HitResult::DropZone {
                target: DropTarget::Tabs {
                    group: target_group,
                    index: 0,
                },
                zone: DropZone::TabInsertion,
            }
        );

        let top_zone = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::Top
                    && zone.target
                        == (DropTarget::GroupEdge {
                            group: target_group,
                            zone: DropZone::Top,
                        })
            })
            .expect("the target group should have a top-edge zone");
        let top_point = Point::new(
            top_zone.rect.origin.x + top_zone.rect.size.width / 2.0,
            top_zone.rect.origin.y + top_zone.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.hit_test(top_point, &drag_rects, &metrics).unwrap(),
            HitResult::DropZone {
                target: DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::Top,
                },
                zone: DropZone::Top,
            }
        );
    }

    #[test]
    fn drop_zone_precedence_makes_rail_insertions_reachable() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, icon_node) = builder.panel(key("drop-rail-icon"), constraints());
        let icon_group = builder.tabs(vec![icon_node], 0, true);
        let rail_column = builder.column(
            metrics.rail_width,
            ColumnMode::IconsOnly,
            vec![icon_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, rail_column);

        let (source_panel, source_node) = builder.panel(key("drop-rail-source"), constraints());
        let source_group = builder.tabs(vec![source_node], 0, false);
        let source_column =
            builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
        builder.main_area(DockSide::Right, source_column);

        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1200.0, 800.0), &metrics)
            .unwrap();
        let zones = layout
            .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
            .unwrap();
        let mut drag_rects = rects.clone();
        drag_rects.drop_zones = zones.clone();

        let rail_zone = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::RailInsertion
                    && zone.target
                        == (DropTarget::RailSlot {
                            column: rail_column,
                            index: 0,
                        })
            })
            .expect("the icon rail should have a first insertion zone");
        let rail_point = Point::new(
            rail_zone.rect.origin.x + rail_zone.rect.size.width / 2.0,
            rail_zone.rect.origin.y + rail_zone.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.hit_test(rail_point, &drag_rects, &metrics).unwrap(),
            HitResult::DropZone {
                target: DropTarget::RailSlot {
                    column: rail_column,
                    index: 0,
                },
                zone: DropZone::RailInsertion,
            }
        );
    }

    #[test]
    fn splitter_priority_beats_overlapping_panel_content() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, first_panel_node) = builder.panel(key("split-left"), constraints());
        let first_group = builder.tabs(vec![first_panel_node], 0, false);
        let first_column =
            builder.column(280.0, ColumnMode::Expanded, vec![first_group], vec![1.0]);

        let (_, second_panel_node) = builder.panel(key("split-right"), constraints());
        let second_group = builder.tabs(vec![second_panel_node], 0, false);
        let second_column =
            builder.column(280.0, ColumnMode::Expanded, vec![second_group], vec![1.0]);

        let split = builder.split(Axis::X, vec![first_column, second_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, split);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1200.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(279.0, 120.0), &rects, &metrics)
                .unwrap(),
            HitResult::Splitter {
                owner: split,
                boundary_index: 0,
                axis: Axis::X
            }
        );
    }

    #[test]
    fn group_header_precedes_tab_header_and_column_background() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel_id, panel_node) = builder.panel(key("header-hit"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(100.0, 10.0), &rects, &metrics)
                .unwrap(),
            HitResult::Group { group }
        );
        assert_eq!(
            layout
                .hit_test(Point::new(100.0, 100.0), &rects, &metrics)
                .unwrap(),
            HitResult::Panel { panel: panel_id }
        );
    }

    #[test]
    fn splitter_hit_zone_far_edge_is_half_open() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, first_panel_node) = builder.panel(key("split-gap-left"), constraints());
        let first_group = builder.tabs(vec![first_panel_node], 0, false);
        let first_column =
            builder.column(280.0, ColumnMode::Expanded, vec![first_group], vec![1.0]);

        let (_, second_panel_node) = builder.panel(key("split-gap-right"), constraints());
        let second_group = builder.tabs(vec![second_panel_node], 0, false);
        let second_column =
            builder.column(280.0, ColumnMode::Expanded, vec![second_group], vec![1.0]);

        let split = builder.split(Axis::X, vec![first_column, second_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, split);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1200.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(285.999, 120.0), &rects, &metrics)
                .unwrap(),
            HitResult::Splitter {
                owner: split,
                boundary_index: 0,
                axis: Axis::X
            }
        );
        assert_ne!(
            layout
                .hit_test(Point::new(286.0, 120.0), &rects, &metrics)
                .unwrap(),
            HitResult::Splitter {
                owner: split,
                boundary_index: 0,
                axis: Axis::X
            }
        );
    }

    #[test]
    fn tab_header_uses_half_open_bounds_and_precedes_panel_content() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (left_panel, left_node) = builder.panel(key("tab-left"), constraints());
        let (right_panel, right_node) = builder.panel(key("tab-right"), constraints());
        let group = builder.tabs(vec![left_node, right_node], 1, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(139.999, 40.0), &rects, &metrics)
                .unwrap(),
            HitResult::TabHeader {
                group,
                panel: left_panel,
                index: 0
            }
        );
        let boundary = Point::new(140.0, 40.0);
        let expected = HitResult::TabHeader {
            group,
            panel: right_panel,
            index: 1,
        };
        for _ in 0..3 {
            assert_eq!(
                layout.hit_test(boundary, &rects, &metrics).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn vertical_overflow_content_is_not_hit_outside_its_viewport() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) = builder.panel(key(&format!("offscreen-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, index == 0));
        }
        let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.2, 0.3, 0.5]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 120.0), &metrics)
            .unwrap();

        assert!(!rects.vertical_overflow.is_empty());
        assert!(
            rects
                .panels
                .iter()
                .any(|panel| panel.rect.origin.y + panel.rect.size.height > 120.0)
        );
        assert_eq!(
            layout
                .hit_test(Point::new(100.0, 130.0), &rects, &metrics)
                .unwrap(),
            HitResult::Miss
        );
    }

    #[test]
    fn canvas_and_empty_space_are_distinct_and_right_bottom_edges_are_exclusive() {
        let metrics = DockMetrics::default();
        let layout = LayoutBuilder::new(Profile::Photoshop).build();
        let rects = layout
            .compute_rects(viewport(1000.0, 600.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(0.0, 0.0), &rects, &metrics)
                .unwrap(),
            HitResult::Canvas
        );
        assert_eq!(
            layout
                .hit_test(Point::new(999.999, 599.999), &rects, &metrics)
                .unwrap(),
            HitResult::Canvas
        );
        assert_eq!(
            layout
                .hit_test(Point::new(1000.0, 300.0), &rects, &metrics)
                .unwrap(),
            HitResult::Miss
        );
        assert_eq!(
            layout
                .hit_test(Point::new(500.0, 600.0), &rects, &metrics)
                .unwrap(),
            HitResult::Miss
        );
        assert!(matches!(
            layout.hit_test(Point::new(f32::NAN, 0.0), &rects, &metrics),
            Err(ComputeRectsError::Geometry(GeometryError::NonFinite))
        ));
    }

    #[test]
    fn collapsed_tab_header_is_hit_without_inventing_a_group_header() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel_id, panel_node) = builder.panel(key("collapsed-hit"), constraints());
        let group = builder.tabs(vec![panel_node], 0, true);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            layout
                .hit_test(Point::new(100.0, 10.0), &rects, &metrics)
                .unwrap(),
            HitResult::TabHeader {
                group,
                panel: panel_id,
                index: 0
            }
        );
    }
}
