use std::collections::HashSet;

use crate::{
    Axis, ComputeRectsError, DockMetrics, DropTarget, DropZone, FloatingGroupId, FloatingGroupRect,
    FlyoutRect, Layout, LayoutRects, Node, NodeId, PanelId, Point, Rect, ResizeEdge, Size,
};

/// Semantic result of hit-testing a point against a LayoutRects snapshot.
///
/// The result contains model handles and interaction meaning only. Pointer
/// events, views, windows, and platform-specific objects belong to the adapter layer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HitResult {
    /// The point belongs to the central application Canvas.
    Canvas,
    /// The point does not belong to any known interactive geometry.
    Miss,
    Column {
        column: NodeId,
    },
    Group {
        group: NodeId,
    },
    TabStrip {
        group: NodeId,
    },
    TabHeader {
        group: NodeId,
        panel: PanelId,
        index: usize,
    },
    /// Button that exposes tabs omitted from the visible tab headers.
    TabOverflowButton {
        group: NodeId,
    },
    Panel {
        panel: PanelId,
    },
    /// The active panel surface inside a transient rail flyout.
    FlyoutPanel {
        column: NodeId,
        group: NodeId,
        panel: PanelId,
    },
    /// The flyout's resize edge facing the Canvas.
    FlyoutResizeEdge {
        column: NodeId,
        panel: PanelId,
        edge: ResizeEdge,
    },
    Splitter {
        owner: NodeId,
        boundary_index: usize,
        axis: Axis,
    },
    /// The title band of a floating window, used for moving the window.
    FloatingTitle {
        group: FloatingGroupId,
    },
    /// The body/background of a floating window without a more specific hit.
    FloatingGroup {
        group: FloatingGroupId,
    },
    FloatingResizeEdge {
        group: FloatingGroupId,
        edge: ResizeEdge,
    },
    DropZone {
        target: DropTarget,
        zone: DropZone,
    },
}

#[cfg(test)]
mod hit_result_tests {
    use slotmap::Key;

    use super::HitResult;
    #[cfg(feature = "serde")]
    use crate::PanelId;
    use crate::{Axis, DropTarget, DropZone, NodeId, ResizeEdge};

    #[test]
    fn hit_results_are_pure_semantic_values() {
        let hit = HitResult::Splitter {
            owner: NodeId::null(),
            boundary_index: 1,
            axis: Axis::Y,
        };
        assert!(matches!(
            hit,
            HitResult::Splitter {
                boundary_index: 1,
                axis: Axis::Y,
                ..
            }
        ));

        let hit = HitResult::DropZone {
            target: DropTarget::Tabs {
                group: NodeId::null(),
                index: 2,
            },
            zone: DropZone::Bottom,
        };
        assert!(matches!(
            hit,
            HitResult::DropZone {
                zone: DropZone::Bottom,
                ..
            }
        ));

        let hit = HitResult::TabOverflowButton {
            group: NodeId::null(),
        };
        assert!(matches!(hit, HitResult::TabOverflowButton { group } if group == NodeId::null()));

        let hit = HitResult::FloatingResizeEdge {
            group: crate::FloatingGroupId::null(),
            edge: ResizeEdge::BottomRight,
        };
        assert!(matches!(
            hit,
            HitResult::FloatingResizeEdge {
                edge: ResizeEdge::BottomRight,
                ..
            }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn hit_result_round_trips_through_serde() {
        let hit = HitResult::TabHeader {
            group: NodeId::null(),
            panel: PanelId::null(),
            index: 3,
        };
        let encoded = serde_json::to_string(&hit).unwrap();
        assert_eq!(serde_json::from_str::<HitResult>(&encoded).unwrap(), hit);

        let overflow = HitResult::TabOverflowButton {
            group: NodeId::null(),
        };
        let encoded = serde_json::to_string(&overflow).unwrap();
        assert_eq!(
            serde_json::from_str::<HitResult>(&encoded).unwrap(),
            overflow
        );
    }
}

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

#[cfg(test)]
#[path = "hit_test_tests.rs"]
mod tests;

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
    match layout
        .node(root)
        .ok_or(ComputeRectsError::InvalidLayoutState)?
    {
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
