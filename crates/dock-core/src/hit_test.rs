use std::collections::HashSet;

use crate::{
    ComputeRectsError, DockMetrics, DropTarget, FloatingGroupRect, GeometryError, HitResult,
    Layout, LayoutRects, Node, NodeId, PanelId, Point, Rect, ResizeEdge,
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
        metrics.validate().map_err(|_| ComputeRectsError::InvalidMetrics)?;
        point.validate().map_err(ComputeRectsError::Geometry)?;

        let floating_order = floating_front_to_back(&rects.floating_groups);
        let (floating_nodes, floating_panels) = all_floating_subtrees(self, &floating_order)?;

        for floating in floating_order {
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
                return Ok(HitResult::FloatingGroup { group: floating.group });
            }

            if let Some((nodes, panels)) = floating_subtree(self, floating.group)? {
                if let Some(hit) = hit_layout_geometry(
                    self, point, rects, metrics, &nodes, &panels, true,
                )? {
                    return Ok(hit);
                }
            }

            // A floating window is opaque for hit dispatch purposes: even blank
            // space in a front window must not activate a dock beneath it.
            return Ok(HitResult::FloatingGroup { group: floating.group });
        }

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

        if let Some(canvas) = rects.canvas {
            if contains(canvas, point)? {
                return Ok(HitResult::Canvas);
            }
        }

        // Keep unknown points distinct from the central Canvas.
        Ok(HitResult::Miss)
    }
}

fn contains(rect: Rect, point: Point) -> Result<bool, ComputeRectsError> {
    rect.contains(point).map_err(ComputeRectsError::Geometry)
}

fn floating_front_to_back(floating: &[FloatingGroupRect]) -> Vec<FloatingGroupRect> {
    let mut ordered = floating.to_vec();
    ordered.sort_by(|left, right| right.z_order.cmp(&left.z_order));
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
    let children = match layout.node(root).ok_or(ComputeRectsError::InvalidLayout)? {
        Node::Split(split) => split.children.clone(),
        Node::Column(column) => column.groups.clone(),
        Node::Tabs(tabs) => tabs.children.clone(),
        Node::Panel(panel) => {
            panels.insert(panel.panel_id);
            Vec::new()
        }
    };
    for child in children {
        collect_subtree(layout, child, nodes, panels)?;
    }
    Ok(())
}

fn floating_subtree(
    layout: &Layout,
    id: crate::FloatingGroupId,
) -> Result<Option<(HashSet<NodeId>, HashSet<PanelId>)>, ComputeRectsError> {
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
        if let Some((subtree_nodes, subtree_panels)) = floating_subtree(layout, item.group)? {
            nodes.extend(subtree_nodes);
            panels.extend(subtree_panels);
        }
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
        if node_in_scope(splitter.owner, scope_nodes, include_only) && contains(splitter.rect, point)? {
            return Ok(Some(HitResult::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis: splitter.axis,
            }));
        }
    }

    // A GroupRect's upper header band is the non-floating group/column header.
    // Collapsed groups have no separate header: their tab strip starts at y=0.
    for group in rects.groups.iter().rev() {
        if !node_in_scope(group.group, scope_nodes, include_only) {
            continue;
        }
        if !rects.tab_strips.iter().any(|strip| strip.group == group.group) {
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

    // Drop zones are semantic overlays on group bodies and win over panel
    // content, but never over resize, splitter or tab-header hit regions.
    for zone in rects.drop_zones.iter().rev() {
        if target_in_scope(zone.target, scope_nodes, include_only) && contains(zone.rect, point)? {
            return Ok(Some(HitResult::DropZone {
                target: zone.target,
                zone: zone.zone,
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
        if node_in_scope(column.column, scope_nodes, include_only) && contains(column.rect, point)? {
            return Ok(Some(HitResult::Column { column: column.column }));
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

fn target_in_scope(
    target: DropTarget,
    scope_nodes: &HashSet<NodeId>,
    include_only: bool,
) -> bool {
    let target_node = match target {
        DropTarget::Tabs { group, .. } | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::RailSlot { column, .. } => Some(column),
        DropTarget::MainColumn { .. } => None,
    };
    target_node.is_some_and(|node| node_in_scope(node, scope_nodes, include_only))
}

#[cfg(test)]
mod tests {
    use super::{ComputeRectsError, GeometryError, HitResult, Point, Rect, ResizeEdge};
    use crate::layout_builder::LayoutBuilder;
    use crate::{
        Axis, ColumnMode, DockMetrics, DockSide, DropTarget, PanelConstraints, PanelKey, Profile,
        Size,
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

    #[test]
    fn every_floating_resize_zone_beats_title_drag() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("resize-zones"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let bounds = Rect::new(Point::new(100.0, 100.0), Size::new(300.0, 200.0));
        let floating = builder.floating_group(group, bounds, 1);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(1000.0, 700.0), &metrics).unwrap();

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
                HitResult::FloatingResizeEdge { group: floating, edge: expected }
            );
        }

        assert_eq!(
            layout.hit_test(Point::new(140.0, 112.0), &rects, &metrics).unwrap(),
            HitResult::FloatingGroup { group: floating }
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
        let rects = layout.compute_rects(viewport(1000.0, 700.0), &metrics).unwrap();

        assert_eq!(
            rects.floating_groups.iter().map(|group| group.z_order).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            layout.hit_test(Point::new(180.0, 170.0), &rects, &metrics).unwrap(),
            HitResult::Panel { panel: upper_panel }
        );
        assert_ne!(lower_panel, upper_panel);

        // The front window's drop zone wins over the overlapping lower window.
        assert_eq!(
            layout.hit_test(Point::new(390.0, 280.0), &rects, &metrics).unwrap(),
            HitResult::DropZone {
                target: DropTarget::GroupEdge { group: upper_group, zone: crate::DropZone::Bottom },
                zone: crate::DropZone::Bottom,
            }
        );
        assert_ne!(lower, upper);
    }

    #[test]
    fn splitter_priority_beats_overlapping_panel_content() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, first_panel_node) = builder.panel(key("split-left"), constraints());
        let first_group = builder.tabs(vec![first_panel_node], 0, false);
        let first_column = builder.column(280.0, ColumnMode::Expanded, vec![first_group], vec![1.0]);

        let (_, second_panel_node) = builder.panel(key("split-right"), constraints());
        let second_group = builder.tabs(vec![second_panel_node], 0, false);
        let second_column = builder.column(280.0, ColumnMode::Expanded, vec![second_group], vec![1.0]);

        let split = builder.split(Axis::X, vec![first_column, second_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, split);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(1200.0, 500.0), &metrics).unwrap();

        assert_eq!(
            layout.hit_test(Point::new(279.0, 120.0), &rects, &metrics).unwrap(),
            HitResult::Splitter { owner: split, boundary_index: 0, axis: Axis::X }
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
        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(
            layout.hit_test(Point::new(139.999, 40.0), &rects, &metrics).unwrap(),
            HitResult::TabHeader { group, panel: left_panel, index: 0 }
        );
        assert_eq!(
            layout.hit_test(Point::new(140.0, 40.0), &rects, &metrics).unwrap(),
            HitResult::TabHeader { group, panel: right_panel, index: 1 }
        );
    }

    #[test]
    fn canvas_and_empty_space_are_distinct_and_right_bottom_edges_are_exclusive() {
        let metrics = DockMetrics::default();
        let layout = LayoutBuilder::new(Profile::Photoshop).build();
        let rects = layout.compute_rects(viewport(1000.0, 600.0), &metrics).unwrap();

        assert_eq!(
            layout.hit_test(Point::new(0.0, 0.0), &rects, &metrics).unwrap(),
            HitResult::Canvas
        );
        assert_eq!(
            layout.hit_test(Point::new(999.999, 599.999), &rects, &metrics).unwrap(),
            HitResult::Canvas
        );
        assert_eq!(
            layout.hit_test(Point::new(1000.0, 300.0), &rects, &metrics).unwrap(),
            HitResult::Miss
        );
        assert_eq!(
            layout.hit_test(Point::new(500.0, 600.0), &rects, &metrics).unwrap(),
            HitResult::Miss
        );
        assert!(matches!(
            layout.hit_test(Point::new(f32::NAN, 0.0), &rects, &metrics),
            Err(ComputeRectsError::Geometry(GeometryError::NonFinite))
        ));
    }

    #[test]
    fn collapsed_tab_strip_is_hit_without_inventing_a_group_header() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("collapsed-hit"), constraints());
        let group = builder.tabs(vec![panel_node], 0, true);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(
            layout.hit_test(Point::new(100.0, 10.0), &rects, &metrics).unwrap(),
            HitResult::TabStrip { group }
        );
    }
}
