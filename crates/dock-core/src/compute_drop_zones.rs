use crate::{
    AllowedArea, ComputeRectsError, DockMetrics, DockSide, DropSource, DropTarget,
    DropZone, DropZoneRect, FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId, Point,
    Profile, Rect, Size,
};

impl Layout {
    /// Builds source-aware drop affordances from an already computed geometry snapshot.
    ///
    /// Only legal targets are emitted. The same snapshot must have been produced by
    /// this Layout; a default/uncomputed snapshot is rejected. Drop zones are separate
    /// from compute_rects because permissions depend on the dragged item.
    pub fn compute_drop_zones(
        &self,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<Vec<DropZoneRect>, ComputeRectsError> {
        metrics.validate().map_err(|_| ComputeRectsError::InvalidMetrics)?;
        self.validate(metrics).map_err(|_| ComputeRectsError::InvalidLayout)?;
        let viewport = rects.viewport.ok_or(ComputeRectsError::InvalidLayout)?;
        viewport.validate_non_empty().map_err(ComputeRectsError::Geometry)?;
        let source_root = source_root(self, source)?;
        let source_panels = collect_panels(self, source_root);
        if source_panels.is_empty()
            || source_panels.iter().any(|panel| self.panel_instance(*panel).is_none())
        {
            return Err(ComputeRectsError::InvalidLayout);
        }

        // Validate the geometry entries we are about to use. A stale target ID
        // is ignored per target, while invalid numeric geometry is a typed error.
        for entry in rects.groups.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.columns.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.tab_strips.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.tab_headers.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.floating_groups.iter() {
            entry.rect.validate_non_empty().map_err(ComputeRectsError::Geometry)?;
        }

        let left_columns = main_columns(self, rects, DockSide::Left);
        let right_columns = main_columns(self, rects, DockSide::Right);
        let may_add_column = can_add_column(
            self,
            source,
            source_root,
            &left_columns,
            &right_columns,
            metrics,
        );
        let mut zones = Vec::new();

        // Main-dock groups: center/tab insertion, vertical group splitting,
        // and profile-correct horizontal/column splitting.
        for group_rect in &rects.groups {
            if !matches!(self.node(group_rect.group), Some(Node::Tabs(_))) {
                continue;
            }
            if is_rail_group(self, group_rect.group) {
                continue;
            }
            let Some(area) = target_area(self, group_rect.group) else {
                continue;
            };
            if area == AllowedArea::FLOATING {
                continue;
            }

            let tabs = match self.node(group_rect.group) {
                Some(Node::Tabs(tabs)) => tabs,
                _ => continue,
            };
            let source_contains_target = matches!(source, DropSource::Group { .. })
                && contains_node(self, source_root, group_rect.group);
            let source_panel_is_here = match source {
                DropSource::Panel { panel } => {
                    panel_node(self, panel)
                        .and_then(|node| direct_parent(self, node))
                        == Some(group_rect.group)
                }
                DropSource::Group { .. } => false,
            };
            if !source_contains_target && !source_panel_is_here
                && source_allows(self, &source_panels, area)
            {
                let center_rect = group_center_rect(group_rect.rect, metrics)?;
                let target = match source {
                    DropSource::Panel { .. } => DropTarget::Tabs {
                        group: group_rect.group,
                        index: tabs.children.len(),
                    },
                    DropSource::Group { .. } => DropTarget::GroupTabs {
                        group: group_rect.group,
                        index: tabs.children.len(),
                    },
                };
                push_zone(&mut zones, target, DropZone::Center, center_rect)?;
            }

            if source_contains_target
                || (matches!(source, DropSource::Group { .. }) && contains_node(self, group_rect.group, source_root))
                || !source_allows(self, &source_panels, area)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Top, DropZone::Top),
                (DropZone::Bottom, DropZone::Bottom),
            ] {
                if group_edge_is_noop(self, source, group_rect.group, edge) {
                    continue;
                }
                let target = DropTarget::GroupEdge {
                    group: group_rect.group,
                    zone: edge,
                };
                push_zone(
                    &mut zones,
                    target,
                    kind,
                    group_edge_rect(group_rect.rect, edge, metrics.edge_drop_width)?,
                )?;
            }

            let left_edge = group_edge_rect(group_rect.rect, DropZone::Left, metrics.edge_drop_width)?;
            let right_edge = group_edge_rect(group_rect.rect, DropZone::Right, metrics.edge_drop_width)?;
            if let Some((side, column_index, _column_rect)) =
                group_main_column(self, group_rect.group, &left_columns, &right_columns)
            {
                if may_add_column {
                    let left_target = match source {
                        DropSource::Panel { .. } => DropTarget::MainColumn {
                            side,
                            index: column_index,
                        },
                        DropSource::Group { .. } => DropTarget::GroupEdge {
                            group: group_rect.group,
                            zone: DropZone::Left,
                        },
                    };
                    let right_target = match source {
                        DropSource::Panel { .. } => DropTarget::MainColumn {
                            side,
                            index: column_index + 1,
                        },
                        DropSource::Group { .. } => DropTarget::GroupEdge {
                            group: group_rect.group,
                            zone: DropZone::Right,
                        },
                    };
                    if !group_edge_is_noop(self, source, group_rect.group, DropZone::Left) {
                        push_zone(&mut zones, left_target, DropZone::ColumnLeft, left_edge)?;
                    }
                    if !group_edge_is_noop(self, source, group_rect.group, DropZone::Right) {
                        push_zone(&mut zones, right_target, DropZone::ColumnRight, right_edge)?;
                    }
                }
            } else if self.profile() == Profile::Generic {
                // Generic layouts permit a nested horizontal Split around a group.
                for (edge, rect) in [
                    (DropZone::Left, left_edge),
                    (DropZone::Right, right_edge),
                ] {
                    if !group_edge_is_noop(self, source, group_rect.group, edge) {
                        push_zone(
                            &mut zones,
                            DropTarget::GroupEdge {
                                group: group_rect.group,
                                zone: edge,
                            },
                            edge,
                            rect,
                        )?;
                    }
                }
            }
        }

        // Tab strips are independent insertion/reorder affordances. Each header
        // is split into two half-open halves mapping to the exact slots before/after it.
        for header in &rects.tab_headers {
            let Some(area) = target_area(self, header.group) else {
                continue;
            };
            if area == AllowedArea::ICON_RAIL || !source_allows(self, &source_panels, area) {
                continue;
            }
            if matches!(source, DropSource::Group { .. })
                && (contains_node(self, source_root, header.group)
                    || contains_node(self, header.group, source_root))
            {
                continue;
            }
            let half = header.rect.size.width / 2.0;
            let left_rect = Rect::new(header.rect.origin, Size::new(half, header.rect.size.height));
            let right_rect = Rect::new(
                Point::new(header.rect.origin.x + half, header.rect.origin.y),
                Size::new(header.rect.size.width - half, header.rect.size.height),
            );
            let target = match source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: header.group,
                    index: header.index,
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: header.group,
                    index: header.index,
                },
            };
            push_zone(&mut zones, target, DropZone::TabInsertion, left_rect)?;
            let right_target = match source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: header.group,
                    index: header.index + 1,
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: header.group,
                    index: header.index + 1,
                },
            };
            push_zone(&mut zones, right_target, DropZone::TabInsertion, right_rect)?;
        }

        // Main-column side targets and append-to-column targets.
        for (side, columns) in [
            (DockSide::Left, &left_columns),
            (DockSide::Right, &right_columns),
        ] {
            for (index, (column_id, column_rect)) in columns.iter().enumerate() {
                let Some(Node::Column(column)) = self.node(*column_id) else {
                    continue;
                };
                if column.mode.is_rail() {
                    continue;
                }
                if !source_allows(self, &source_panels, AllowedArea::MAIN_DOCK) {
                    continue;
                }
                let bottom = column_rect.checked_bottom().map_err(ComputeRectsError::Geometry)?;
                let end_height = metrics.edge_drop_width.min(column_rect.size.height / 3.0);
                let side_inset = metrics.edge_drop_width.min(column_rect.size.width / 4.0);
                let end_rect = Rect::new(
                    Point::new(column_rect.origin.x + side_inset, bottom - end_height),
                    Size::new((column_rect.size.width - 2.0 * side_inset).max(0.0), end_height),
                );
                let end_index = column.groups.len();
                if !group_slot_is_noop(self, source, *column_id, end_index) {
                    push_zone(
                        &mut zones,
                        DropTarget::ColumnSlot {
                            column: *column_id,
                            index: end_index,
                        },
                        DropZone::ColumnEnd,
                        end_rect,
                    )?;
                }

                // The top/bottom halves of the column boundary zones are represented
                // by group targets; the outer side itself can safely use full height.
                let edge_width = metrics.edge_drop_width.min(column_rect.size.width / 3.0);
                let left_rect = Rect::new(
                    column_rect.origin,
                    Size::new(edge_width, column_rect.size.height),
                );
                let right_rect = Rect::new(
                    Point::new(column_rect.origin.x + column_rect.size.width - edge_width, column_rect.origin.y),
                    Size::new(edge_width, column_rect.size.height),
                );
                if may_add_column {
                    let targets = match source {
                        DropSource::Panel { .. } => Some((
                            DropTarget::MainColumn { side, index },
                            DropTarget::MainColumn { side, index: index + 1 },
                        )),
                        DropSource::Group { .. } => edge_group_for_column(self, *column_id, false)
                            .zip(edge_group_for_column(self, *column_id, true))
                            .map(|(left_group, right_group)| (
                                DropTarget::GroupEdge { group: left_group, zone: DropZone::Left },
                                DropTarget::GroupEdge { group: right_group, zone: DropZone::Right },
                            )),
                    };
                    if let Some((left_target, right_target)) = targets {
                        let left_is_noop = match &left_target {
                            DropTarget::GroupEdge { group, zone } => {
                                group_edge_is_noop(self, source, *group, *zone)
                            }
                            _ => false,
                        };
                        let right_is_noop = match &right_target {
                            DropTarget::GroupEdge { group, zone } => {
                                group_edge_is_noop(self, source, *group, *zone)
                            }
                            _ => false,
                        };
                        if !left_is_noop {
                            push_zone(&mut zones, left_target, DropZone::ColumnLeft, left_rect)?;
                        }
                        if !right_is_noop {
                            push_zone(&mut zones, right_target, DropZone::ColumnRight, right_rect)?;
                        }
                    }
                }
            }
        }

        // Exact insertion points in icon rails follow each rail's flattened group order.
        for column_rect in &rects.columns {
            let column_id = column_rect.column;
            let Some(Node::Column(column)) = self.node(column_id) else {
                continue;
            };
            let is_main_column = left_columns.iter().any(|(id, _)| *id == column_id)
                || right_columns.iter().any(|(id, _)| *id == column_id);
            if !column.mode.is_rail() || !is_main_column {
                continue;
            }
            if !source_allows(self, &source_panels, AllowedArea::ICON_RAIL) {
                continue;
            }
            let mut icons: Vec<_> = rects.tab_headers.iter()
                .filter(|icon| column.groups.iter().any(|root| contains_node(self, *root, icon.group)))
                .collect();
            icons.sort_by(|left, right| {
                left.rect.origin.y.partial_cmp(&right.rect.origin.y).unwrap_or(std::cmp::Ordering::Equal)
            });
            if icons.is_empty() {
                push_zone(
                    &mut zones,
                    DropTarget::RailSlot { column: column_id, index: 0 },
                    DropZone::RailInsertion,
                    column_rect.rect,
                )?;
            } else {
                for (index, icon) in icons.iter().enumerate() {
                    let top_height = icon.rect.size.height / 2.0;
                    push_zone(
                        &mut zones,
                        DropTarget::RailSlot { column: column_id, index },
                        DropZone::RailInsertion,
                        Rect::new(icon.rect.origin, Size::new(icon.rect.size.width, top_height)),
                    )?;
                    push_zone(
                        &mut zones,
                        DropTarget::RailSlot { column: column_id, index: index + 1 },
                        DropZone::RailInsertion,
                        Rect::new(
                            Point::new(icon.rect.origin.x, icon.rect.origin.y + top_height),
                            Size::new(icon.rect.size.width, icon.rect.size.height - top_height),
                        ),
                    )?;
                }
            }
        }

        // Floating-window drop zones are stacked over their contents. Their outer
        // top/bottom targets address the first/last group in a vertical stack.
        for floating_rect in rects.floating_groups.iter().rev() {
            let Some(floating) = self.floating_group(floating_rect.group) else {
                continue;
            };
            let Some(area) = self.dock_area(floating.area) else {
                continue;
            };
            if !source_allows(self, &source_panels, AllowedArea::FLOATING) {
                continue;
            }
            let mut target_groups: Vec<_> = rects.groups.iter()
                .filter(|group| contains_node(self, area.root, group.group))
                .collect();
            target_groups.sort_by(|left, right| {
                left.rect.origin.y.partial_cmp(&right.rect.origin.y).unwrap_or(std::cmp::Ordering::Equal)
            });
            if target_groups.is_empty() {
                continue;
            }
            let center_y = floating_rect.rect.origin.y + floating_rect.rect.size.height / 2.0;
            let center_group = target_groups.iter().min_by(|left, right| {
                let left_y = left.rect.origin.y + left.rect.size.height / 2.0;
                let right_y = right.rect.origin.y + right.rect.size.height / 2.0;
                (left_y - center_y).abs().partial_cmp(&(right_y - center_y).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }).copied();
            if let Some(center_group) = center_group {
                let is_self = matches!(source, DropSource::Group { .. })
                    && contains_node(self, source_root, center_group.group);
                let panel_already_in_group = match source {
                    DropSource::Panel { panel } => panel_node(self, panel)
                        .and_then(|node_id| direct_parent(self, node_id))
                        == Some(center_group.group),
                    DropSource::Group { .. } => false,
                };
                if !is_self && !panel_already_in_group {
                    let tabs = match self.node(center_group.group) {
                        Some(Node::Tabs(tabs)) => tabs,
                        _ => continue,
                    };
                    let target = match source {
                        DropSource::Panel { .. } => DropTarget::Tabs {
                            group: center_group.group,
                            index: tabs.children.len(),
                        },
                        DropSource::Group { .. } => DropTarget::GroupTabs {
                            group: center_group.group,
                            index: tabs.children.len(),
                        },
                    };
                    let bounds = floating_rect.rect;
                    let left = metrics.edge_drop_width.min(bounds.size.width / 3.0);
                    let top = bounds.origin.y + metrics.header_height + metrics.tab_strip_height;
                    let bottom = bounds.origin.y + bounds.size.height
                        - metrics.edge_drop_width.min(bounds.size.height / 3.0);
                    let center_rect = Rect::new(
                        Point::new(left + bounds.origin.x, top),
                        Size::new((bounds.size.width - 2.0 * left).max(0.0), (bottom - top).max(0.0)),
                    );
                    push_zone(&mut zones, target, DropZone::FloatingCenter, center_rect)?;
                }
            }

            let first_group = target_groups.first().copied().unwrap();
            let last_group = target_groups.last().copied().unwrap();
            let top_rect = floating_edge_rect(floating_rect.rect, DropZone::Top, metrics)?;
            let bottom_rect = floating_edge_rect(floating_rect.rect, DropZone::Bottom, metrics)?;
            if !(matches!(source, DropSource::Group { .. })
                && group_edge_is_noop(self, source, first_group.group, DropZone::Top))
            {
                push_zone(
                    &mut zones,
                    DropTarget::GroupEdge { group: first_group.group, zone: DropZone::Top },
                    DropZone::FloatingTop,
                    top_rect,
                )?;
            }
            if !(matches!(source, DropSource::Group { .. })
                && group_edge_is_noop(self, source, last_group.group, DropZone::Bottom))
            {
                push_zone(
                    &mut zones,
                    DropTarget::GroupEdge { group: last_group.group, zone: DropZone::Bottom },
                    DropZone::FloatingBottom,
                    bottom_rect,
                )?;
            }

            // Generic profile also supports horizontal nested splits within a floating area.
            if self.profile() == Profile::Generic {
                for target_group in target_groups {
                    if matches!(source, DropSource::Group { .. })
                        && contains_node(self, source_root, target_group.group)
                    {
                        continue;
                    }
                    for (edge, kind) in [(DropZone::Left, DropZone::Left), (DropZone::Right, DropZone::Right)] {
                        if group_edge_is_noop(self, source, target_group.group, edge) {
                            continue;
                        }
                        push_zone(
                            &mut zones,
                            DropTarget::GroupEdge { group: target_group.group, zone: edge },
                            kind,
                            group_edge_rect(target_group.rect, edge, metrics.edge_drop_width)?,
                        )?;
                    }
                }
            }
        }

        // The two outside viewport edges open/extend a dock. A whole group uses
        // its edge operation when a neighbouring column exists; panels use an exact slot.
        if source_allows(self, &source_panels, AllowedArea::MAIN_DOCK) && may_add_column {
            let edge_width = metrics.edge_drop_width.min(viewport.size.width / 3.0);
            for (side, columns, zone, is_left) in [
                (DockSide::Left, &left_columns, DropZone::DockEdge, true),
                (DockSide::Right, &right_columns, DropZone::DockEdge, false),
            ] {
                let rect = if is_left {
                    Rect::new(viewport.origin, Size::new(edge_width, viewport.size.height))
                } else {
                    Rect::new(
                        Point::new(viewport.origin.x + viewport.size.width - edge_width, viewport.origin.y),
                        Size::new(edge_width, viewport.size.height),
                    )
                };
                let target = match source {
                    DropSource::Panel { .. } => Some(DropTarget::MainColumn {
                        side,
                        index: if is_left { 0 } else { columns.len() },
                    }),
                    DropSource::Group { .. } => edge_group_for_dock_edge(self, columns, is_left)
                        .map(|group| DropTarget::GroupEdge {
                            group,
                            zone: if is_left { DropZone::Left } else { DropZone::Right },
                        }),
                };
                if let Some(target) = target {
                    let is_noop = match target {
                        DropTarget::GroupEdge { group, zone } => {
                            group_edge_is_noop(self, source, group, zone)
                        }
                        _ => false,
                    };
                    if !is_noop {
                        push_zone(&mut zones, target, zone, rect)?;
                    }
                }
            }
        }

        // A free Canvas is a Float command hint, never a fake tab/column destination.
        if source_allows(self, &source_panels, AllowedArea::FLOATING)
            && !source_is_floating_root(self, source_root)
        {
            if let Some(canvas) = rects.canvas {
                push_zone(&mut zones, DropTarget::FreeArea, DropZone::FreeArea, canvas)?;
            }
        }

        Ok(zones)
    }

    /// Resolves the point to the highest-priority legal destination, if any.
    ///
    /// Floating windows are handled front-to-back: a point inside the foremost
    /// window never falls through to a lower window or the Canvas.
    pub fn resolve_drop(
        &self,
        point: Point,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<Option<DropTarget>, ComputeRectsError> {
        point.validate().map_err(ComputeRectsError::Geometry)?;
        let zones = self.compute_drop_zones(source, rects, metrics)?;

        let mut topmost_floating: Option<(FloatingGroupId, Rect, i32)> = None;
        for floating in &rects.floating_groups {
            if floating.rect.contains(point).map_err(ComputeRectsError::Geometry)? {
                let replace = topmost_floating
                    .as_ref()
                    .map(|(_, _, z)| floating.z_order > *z)
                    .unwrap_or(true);
                if replace {
                    topmost_floating = Some((floating.group, floating.rect, floating.z_order));
                }
            }
        }
        if let Some((floating_id, _bounds, _z_order)) = topmost_floating {
            let Some(floating) = self.floating_group(floating_id) else {
                return Ok(None);
            };
            let Some(area) = self.dock_area(floating.area) else {
                return Ok(None);
            };
            return best_containing_zone(&zones, point, |zone| {
                matches!(
                    zone.zone,
                    DropZone::FloatingCenter | DropZone::FloatingTop | DropZone::FloatingBottom
                        | DropZone::TabInsertion | DropZone::Left | DropZone::Right
                ) && target_node(zone.target)
                    .map(|node| contains_node(self, area.root, node))
                    .unwrap_or(false)
            });
        }
        best_containing_zone(&zones, point, |_| true)
    }
}

fn push_zone(
    zones: &mut Vec<DropZoneRect>,
    target: DropTarget,
    zone: DropZone,
    rect: Rect,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return Ok(());
    }
    zones.push(DropZoneRect::new(target, zone, rect));
    Ok(())
}

fn best_containing_zone(
    zones: &[DropZoneRect],
    point: Point,
    include: impl Fn(&DropZoneRect) -> bool,
) -> Result<Option<DropTarget>, ComputeRectsError> {
    let mut best: Option<(u8, usize, DropTarget)> = None;
    for (index, zone) in zones.iter().enumerate() {
        if !include(zone) || !zone.rect.contains(point).map_err(ComputeRectsError::Geometry)? {
            continue;
        }
        let priority = zone_priority(zone.zone);
        if best.map(|(best_priority, _, _)| priority < best_priority).unwrap_or(true) {
            best = Some((priority, index, zone.target));
        }
    }
    Ok(best.map(|(_, _, target)| target))
}

fn zone_priority(zone: DropZone) -> u8 {
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

fn source_root(layout: &Layout, source: DropSource) -> Result<NodeId, ComputeRectsError> {
    match source {
        DropSource::Panel { panel } => panel_node(layout, panel).ok_or(ComputeRectsError::InvalidLayout),
        DropSource::Group { group } => {
            if matches!(layout.node(group), Some(Node::Tabs(_))) {
                Ok(group)
            } else {
                Err(ComputeRectsError::InvalidLayout)
            }
        }
    }
}

fn panel_node(layout: &Layout, panel: PanelId) -> Option<NodeId> {
    layout.nodes().find_map(|(node_id, node)| match node {
        Node::Panel(value) if value.panel_id == panel => Some(node_id),
        _ => None,
    })
}

fn collect_panels(layout: &Layout, root: NodeId) -> Vec<PanelId> {
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(node_id) = stack.pop() {
        match layout.node(node_id) {
            Some(Node::Panel(panel)) => result.push(panel.panel_id),
            Some(Node::Tabs(tabs)) => stack.extend(tabs.children.iter().copied()),
            Some(Node::Split(split)) => stack.extend(split.children.iter().copied()),
            Some(Node::Column(column)) => stack.extend(column.groups.iter().copied()),
            None => {}
        }
    }
    result
}

fn source_allows(layout: &Layout, panels: &[PanelId], area: AllowedArea) -> bool {
    panels.iter().all(|panel| {
        layout.panel_instance(*panel)
            .map(|instance| instance.constraints.allows_area(area))
            .unwrap_or(false)
    })
}

fn children(layout: &Layout, node: NodeId) -> Vec<NodeId> {
    match layout.node(node) {
        Some(Node::Tabs(tabs)) => tabs.children.clone(),
        Some(Node::Split(split)) => split.children.clone(),
        Some(Node::Column(column)) => column.groups.clone(),
        Some(Node::Panel(_)) | None => Vec::new(),
    }
}

fn contains_node(layout: &Layout, root: NodeId, target: NodeId) -> bool {
    if root == target {
        return true;
    }
    let mut stack = vec![root];
    while let Some(node_id) = stack.pop() {
        if node_id == target {
            return true;
        }
        stack.extend(children(layout, node_id));
    }
    false
}

fn direct_parent(layout: &Layout, child: NodeId) -> Option<NodeId> {
    layout.nodes().find_map(|(parent, _)| {
        if children(layout, parent).contains(&child) {
            Some(parent)
        } else {
            None
        }
    })
}

fn target_area(layout: &Layout, target: NodeId) -> Option<AllowedArea> {
    if is_rail_group(layout, target) {
        return Some(AllowedArea::ICON_RAIL);
    }
    if floating_owner(layout, target).is_some() {
        return Some(AllowedArea::FLOATING);
    }
    main_side_for_node(layout, target).map(|_| AllowedArea::MAIN_DOCK)
}

fn is_rail_group(layout: &Layout, group: NodeId) -> bool {
    matches!(
        owner_column(layout, group).and_then(|column| layout.node(column)),
        Some(Node::Column(column)) if column.mode.is_rail()
    )
}

fn owner_column(layout: &Layout, node: NodeId) -> Option<NodeId> {
    let mut current = node;
    while let Some(parent) = direct_parent(layout, current) {
        if matches!(layout.node(parent), Some(Node::Column(_))) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

fn floating_owner(layout: &Layout, node: NodeId) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group_id, floating)| {
        layout.dock_area(floating.area)
            .filter(|area| contains_node(layout, area.root, node))
            .map(|_| group_id)
    })
}

fn source_is_floating_root(layout: &Layout, source_root: NodeId) -> bool {
    layout.floating_groups().any(|(_, floating)| {
        layout.dock_area(floating.area).map(|area| area.root == source_root).unwrap_or(false)
    })
}

fn main_side_for_node(layout: &Layout, node: NodeId) -> Option<DockSide> {
    [DockSide::Left, DockSide::Right].into_iter().find(|side| {
        layout.main_area(*side)
            .map(|area| contains_node(layout, area.root, node))
            .unwrap_or(false)
    })
}

fn main_columns(
    layout: &Layout,
    rects: &LayoutRects,
    side: DockSide,
) -> Vec<(NodeId, Rect)> {
    let Some(root) = layout.main_area(side).map(|area| area.root) else {
        return Vec::new();
    };
    // Only root Columns or direct Column children of a horizontal main Split
    // are valid MainColumn/ColumnSlot/RailSlot targets in the command model.
    let column_ids: Vec<NodeId> = match layout.node(root) {
        Some(Node::Column(_)) => vec![root],
        Some(Node::Split(split))
            if split.axis == crate::Axis::X
                && split.children.iter().all(|child| matches!(layout.node(*child), Some(Node::Column(_)))) =>
        {
            split.children.clone()
        }
        _ => return Vec::new(),
    };
    column_ids.into_iter().filter_map(|column_id| {
        rects.columns.iter()
            .find(|column| column.column == column_id)
            .map(|column| (column_id, column.rect))
    }).collect()
}

fn group_main_column(
    layout: &Layout,
    group: NodeId,
    left_columns: &[(NodeId, Rect)],
    right_columns: &[(NodeId, Rect)],
) -> Option<(DockSide, usize, Rect)> {
    let column_id = owner_column(layout, group)?;
    let (side, columns) = if left_columns.iter().any(|(id, _)| *id == column_id) {
        (DockSide::Left, left_columns)
    } else if right_columns.iter().any(|(id, _)| *id == column_id) {
        (DockSide::Right, right_columns)
    } else {
        return None;
    };
    columns.iter().position(|(id, _)| *id == column_id)
        .map(|index| (side, index, columns[index].1))
}

fn edge_group_for_column(layout: &Layout, column: NodeId, last: bool) -> Option<NodeId> {
    let Some(Node::Column(value)) = layout.node(column) else {
        return None;
    };
    if last {
        value.groups.last().copied()
    } else {
        value.groups.first().copied()
    }
}

fn edge_group_for_dock_edge(
    layout: &Layout,
    columns: &[(NodeId, Rect)],
    is_left: bool,
) -> Option<NodeId> {
    let (column, _) = if is_left {
        columns.first().copied()?
    } else {
        columns.last().copied()?
    };
    edge_group_for_column(layout, column, is_left)
}

fn can_add_column(
    layout: &Layout,
    source: DropSource,
    source_root: NodeId,
    left_columns: &[(NodeId, Rect)],
    right_columns: &[(NodeId, Rect)],
    metrics: &DockMetrics,
) -> bool {
    if layout.profile() != Profile::Photoshop
        || left_columns.len() + right_columns.len() < metrics.max_columns
    {
        return true;
    }
    let Some(column_id) = owner_column(layout, source_root) else {
        return false;
    };
    if main_side_for_node(layout, column_id).is_none() {
        return false;
    }
    let Some(Node::Column(column)) = layout.node(column_id) else {
        return false;
    };
    match source {
        DropSource::Group { group } => column.groups.len() == 1 && column.groups[0] == group,
        DropSource::Panel { panel } => {
            let Some(panel_node) = panel_node(layout, panel) else {
                return false;
            };
            let Some(group) = direct_parent(layout, panel_node) else {
                return false;
            };
            column.groups.len() == 1
                && column.groups[0] == group
                && matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.children.len() == 1)
        }
    }
}

fn group_edge_rect(rect: Rect, zone: DropZone, edge_width: f32) -> Result<Rect, ComputeRectsError> {
    let x_band = edge_width.min(rect.size.width / 2.0);
    let y_band = edge_width.min(rect.size.height / 2.0);
    let side_height = (rect.size.height - 2.0 * y_band).max(0.0);
    let result = match zone {
        DropZone::Left => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + y_band),
            Size::new(x_band, side_height),
        ),
        DropZone::Right => Rect::new(
            Point::new(rect.origin.x + rect.size.width - x_band, rect.origin.y + y_band),
            Size::new(x_band, side_height),
        ),
        DropZone::Top => Rect::new(rect.origin, Size::new(rect.size.width, y_band)),
        DropZone::Bottom => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height - y_band),
            Size::new(rect.size.width, y_band),
        ),
        _ => return Err(ComputeRectsError::InvalidLayout),
    };
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

fn group_center_rect(rect: Rect, metrics: &DockMetrics) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.edge_drop_width.min(rect.size.width / 3.0);
    let content_top = (metrics.header_height + metrics.tab_strip_height).min(rect.size.height);
    let inset_bottom = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, rect.origin.y + content_top),
        Size::new(
            (rect.size.width - 2.0 * inset_x).max(0.0),
            (rect.size.height - content_top - inset_bottom).max(0.0),
        ),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

fn floating_edge_rect(
    rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.splitter_hit_zone.min(rect.size.width / 3.0);
    let height = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let y = match edge {
        // Keep stack insertion below the floating title and tab-strip hit bands.
        DropZone::Top => rect.origin.y + metrics.header_height + metrics.tab_strip_height,
        DropZone::Bottom => rect.origin.y + rect.size.height - height,
        _ => return Err(ComputeRectsError::InvalidLayout),
    };
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, y),
        Size::new((rect.size.width - 2.0 * inset_x).max(0.0), height),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

fn group_edge_is_noop(
    layout: &Layout,
    source: DropSource,
    target_group: NodeId,
    zone: DropZone,
) -> bool {
    let DropSource::Group { group: source_group } = source else {
        return false;
    };
    if source_group == target_group || contains_node(layout, source_group, target_group) {
        return true;
    }
    let source_parent = direct_parent(layout, source_group);
    let target_parent = direct_parent(layout, target_group);
    if source_parent.is_some() && source_parent == target_parent {
        let parent = source_parent.unwrap();
        let siblings = match layout.node(parent) {
            Some(Node::Column(column)) => Some(&column.groups),
            Some(Node::Split(split)) if split.axis == crate::Axis::Y => Some(&split.children),
            _ => None,
        };
        if let Some(siblings) = siblings {
            let source_index = siblings.iter().position(|id| *id == source_group);
            let target_index = siblings.iter().position(|id| *id == target_group);
            if let (Some(source_index), Some(target_index)) = (source_index, target_index) {
                if (zone == DropZone::Top && source_index + 1 == target_index)
                    || (zone == DropZone::Bottom && target_index + 1 == source_index)
                {
                    return true;
                }
            }
        }
    }
    if matches!(zone, DropZone::Left | DropZone::Right) {
        let Some(source_column) = owner_column(layout, source_group) else {
            return false;
        };
        let Some(target_column) = owner_column(layout, target_group) else {
            return false;
        };
        if source_column == target_column {
            return false;
        }
        let (Some(source_side), Some(target_side)) = (
            main_side_for_node(layout, source_column),
            main_side_for_node(layout, target_column),
        ) else {
            return false;
        };
        if source_side != target_side {
            return false;
        }
        let root = match layout.main_area(source_side) {
            Some(area) => area.root,
            None => return false,
        };
        let mut columns = Vec::new();
        collect_main_columns(layout, root, &mut columns);
        let source_index = columns.iter().position(|id| *id == source_column);
        let target_index = columns.iter().position(|id| *id == target_column);
        if let (Some(source_index), Some(target_index)) = (source_index, target_index) {
            let source_is_single = matches!(
                layout.node(source_column),
                Some(Node::Column(column)) if column.groups.len() == 1 && column.groups[0] == source_group
            );
            return source_is_single
                && ((zone == DropZone::Left && source_index + 1 == target_index)
                    || (zone == DropZone::Right && target_index + 1 == source_index));
        }
    }
    false
}

fn collect_main_columns(layout: &Layout, root: NodeId, output: &mut Vec<NodeId>) {
    match layout.node(root) {
        Some(Node::Column(_)) => output.push(root),
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_main_columns(layout, *child, output);
            }
        }
        Some(Node::Tabs(_) | Node::Panel(_)) | None => {}
    }
}

fn group_slot_is_noop(layout: &Layout, source: DropSource, column: NodeId, index: usize) -> bool {
    let DropSource::Group { group } = source else {
        return false;
    };
    let Some(Node::Column(value)) = layout.node(column) else {
        return false;
    };
    let Some(source_index) = value.groups.iter().position(|id| *id == group) else {
        return false;
    };
    index == source_index || index == source_index + 1
}

fn target_node(target: DropTarget) -> Option<NodeId> {
    match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::ColumnSlot { column, .. } | DropTarget::RailSlot { column, .. } => Some(column),
        DropTarget::MainColumn { .. } | DropTarget::FreeArea => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::Key;
    use crate::layout_builder::LayoutBuilder;
    use crate::{AllowedArea, ColumnMode, DockCommand, DockMetrics, DockSide, DropSource, DropTarget, DropZone, PanelConstraints, PanelKey, Point, Profile, Rect, Size};

    fn key(name: &str) -> PanelKey {
        PanelKey::new(name).unwrap()
    }

    fn constraints() -> PanelConstraints {
        PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 140.0),
            None,
            false,
        )
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
        let column = builder.column(280.0, ColumnMode::Expanded, vec![source_group, target_group], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        assert!(rects.drop_zones.is_empty());
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();

        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::Tabs { group: target_group, index: 1 }
                && zone.zone == DropZone::Center
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::Tabs { group: target_group, index: 0 }
                && zone.zone == DropZone::TabInsertion
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::Tabs { group: target_group, index: 1 }
                && zone.zone == DropZone::TabInsertion
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::GroupEdge { group: target_group, zone: DropZone::Top }
                && zone.zone == DropZone::Top
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::GroupEdge { group: target_group, zone: DropZone::Bottom }
                && zone.zone == DropZone::Bottom
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::MainColumn { side: DockSide::Left, index: 0 }
                && zone.zone == DropZone::ColumnLeft
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::MainColumn { side: DockSide::Left, index: 1 }
                && zone.zone == DropZone::ColumnRight
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::ColumnSlot { column, index: 2 }
                && zone.zone == DropZone::ColumnEnd
        }));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::MainColumn { side: DockSide::Left, index: 0 }
                && zone.zone == DropZone::DockEdge
        }));
        let left_drop_edge = zones.iter().find(|zone| {
            zone.target == DropTarget::MainColumn { side: DockSide::Left, index: 0 }
                && zone.zone == DropZone::ColumnLeft
        }).unwrap();
        assert_eq!(left_drop_edge.rect.size.width, 24.0);

        let mut wide_metrics = metrics;
        wide_metrics.edge_drop_width = 32.0;
        let wide_rects = layout.compute_rects(viewport(), &wide_metrics).unwrap();
        let wide_zones = layout.compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &wide_rects,
            &wide_metrics,
        ).unwrap();
        let wide_left = wide_zones.iter().find(|zone| {
            zone.target == DropTarget::MainColumn { side: DockSide::Left, index: 0 }
                && zone.zone == DropZone::ColumnLeft
        }).unwrap();
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
        let column = builder.column(280.0, ColumnMode::Expanded, vec![first, second], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, column);
        let mut layout = builder.build();
        let before_self_drop = format!("{layout:?}");
        let before_version = layout.version();
        layout.apply(
            DockCommand::MergeGroup {
                group: first,
                target_group: first,
                index: 0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(format!("{layout:?}"), before_self_drop);
        assert_eq!(layout.version(), before_version);

        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        let zones = layout.compute_drop_zones(DropSource::Group { group: first }, &rects, &metrics).unwrap();

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
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();

        assert!(!zones.iter().any(|zone| matches!(
            zone.target,
            DropTarget::Tabs { group, .. } | DropTarget::GroupEdge { group, .. }
                if group == source_group
        )));
        assert!(zones.iter().any(|zone| {
            zone.target == DropTarget::Tabs { group: float_tabs, index: 1 }
                && zone.zone == DropZone::FloatingCenter
        }));
        assert!(zones.iter().any(|zone| zone.target == DropTarget::FreeArea && zone.zone == DropZone::FreeArea));
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
        let rail = builder.column(metrics.rail_width, ColumnMode::IconsOnly, vec![rail_group], vec![1.0]);
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
            DropTarget::Tabs { group: float_group, index: 1 },
            DropTarget::RailSlot { column: rail, index: 1 },
        ] {
            let before = format!("{layout:?}");
            let version = layout.version();
            assert!(layout.apply(DockCommand::MovePanel { panel: source_panel, target }, &metrics).is_err());
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), version);
        }

        for command in [
            DockCommand::MoveGroup { group: source_group, column: rail, index: 1 },
            DockCommand::SetColumnMode { column: source_column, mode: ColumnMode::IconsOnly },
            DockCommand::DockFloating {
                group: float_id,
                target: DropTarget::Tabs { group: source_group, index: 1 },
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
        assert!(layout.apply(DockCommand::OpenPanel {
            key: key("forged-floating-only"),
            constraints: constraints().with_allowed_areas(AllowedArea::FLOATING),
            target: Some(DropTarget::Tabs { group: source_group, index: 1 }),
        }, &metrics).is_err());
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
        let column = builder.column(280.0, ColumnMode::Expanded, vec![source_group, stale_group], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, column);
        let mut layout = builder.build();

        layout.apply(DockCommand::CloseGroup { group: stale_group }, &metrics).unwrap();
        assert!(layout.node(stale_group).is_none());
        let before = format!("{layout:?}");
        let version = layout.version();
        assert!(layout.apply(
            DockCommand::MovePanel {
                panel: source_panel,
                target: DropTarget::Tabs { group: stale_group, index: 0 },
            },
            &metrics,
        ).is_err());
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
        let zones = layout.compute_drop_zones(
            DropSource::Group { group: source_group },
            &rects,
            &metrics,
        ).unwrap();

        for (zone_kind, edge) in [(DropZone::Left, DropZone::Left), (DropZone::Right, DropZone::Right)] {
            assert!(zones.iter().any(|zone| {
                zone.zone == zone_kind
                    && zone.target == DropTarget::GroupEdge { group: target_group, zone: edge }
            }));
        }
        let left = zones.iter().find(|zone| {
            zone.zone == DropZone::Left
                && zone.target == DropTarget::GroupEdge { group: target_group, zone: DropZone::Left }
        }).unwrap();
        let point = Point::new(
            left.rect.origin.x + left.rect.size.width / 2.0,
            left.rect.origin.y + left.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.resolve_drop(point, DropSource::Group { group: source_group }, &rects, &metrics).unwrap(),
            Some(DropTarget::GroupEdge { group: target_group, zone: DropZone::Left })
        );
    }

    #[test]
    fn max_columns_hides_new_column_targets_when_the_source_column_cannot_collapse() {
        let mut metrics = DockMetrics::default();
        metrics.max_columns = 1;
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (source_panel, source_node) = builder.panel(key("max-column-source"), constraints());
        let (_, other_node) = builder.panel(key("max-column-other"), constraints());
        let group = builder.tabs(vec![source_node, other_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();
        assert!(!zones.iter().any(|zone| matches!(
            zone.target,
            DropTarget::MainColumn { .. } if matches!(zone.zone, DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge)
        )));
    }

    #[test]
    fn floating_center_and_edges_resolve_to_tab_or_stack_targets() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
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
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();

        assert!(zones.iter().any(|zone| {
            zone.zone == DropZone::FloatingCenter
                && zone.target == DropTarget::Tabs { group: float_group, index: 1 }
        }));
        assert!(zones.iter().any(|zone| {
            zone.zone == DropZone::FloatingTop
                && zone.target == DropTarget::GroupEdge { group: float_group, zone: DropZone::Top }
        }));
        assert!(zones.iter().any(|zone| {
            zone.zone == DropZone::FloatingBottom
                && zone.target == DropTarget::GroupEdge { group: float_group, zone: DropZone::Bottom }
        }));

        let center = zones.iter().find(|zone| zone.zone == DropZone::FloatingCenter).unwrap();
        let center_point = Point::new(
            center.rect.origin.x + center.rect.size.width / 2.0,
            center.rect.origin.y + center.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.resolve_drop(center_point, DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap(),
            Some(DropTarget::Tabs { group: float_group, index: 1 })
        );

        let top = zones.iter().find(|zone| zone.zone == DropZone::FloatingTop).unwrap();
        let top_point = Point::new(
            top.rect.origin.x + top.rect.size.width / 2.0,
            top.rect.origin.y + top.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.resolve_drop(top_point, DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap(),
            Some(DropTarget::GroupEdge { group: float_group, zone: DropZone::Top })
        );
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
        let rail = builder.column(metrics.rail_width, ColumnMode::IconsOnly, vec![first, second], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, rail);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();

        for index in 0..=2 {
            assert!(zones.iter().any(|zone| {
                zone.target == DropTarget::RailSlot { column: rail, index }
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
        let rail = builder.column(metrics.min_column_width, ColumnMode::IconsAndLabels, vec![first, second], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, rail);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();

        for index in 0..=2 {
            assert!(zones.iter().any(|zone| {
                zone.target == DropTarget::RailSlot { column: rail, index }
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

        let layout = builder.build();
        let rects = layout.compute_rects(viewport(), &metrics).unwrap();
        let inside = Point::new(600.0, 405.0);
        let resolved = layout.resolve_drop(inside, DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();
        assert!(matches!(
            resolved,
            Some(DropTarget::GroupEdge { group, zone: DropZone::Bottom }) if group == upper_group
        ));
        for _ in 0..4 {
            assert_eq!(
                layout.resolve_drop(
                    inside,
                    DropSource::Panel { panel: source_panel },
                    &rects,
                    &metrics,
                ).unwrap(),
                resolved,
                "drop resolution must be deterministic for identical geometry and input"
            );
        }
        assert_ne!(lower_group, upper_group);
        assert!(layout.resolve_drop(Point::new(f32::NAN, 0.0), DropSource::Panel { panel: source_panel }, &rects, &metrics).is_err());
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
        let zones = layout.compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics).unwrap();
        assert!(!zones.iter().any(|zone| {
            matches!(zone.target, DropTarget::GroupEdge { group, zone: DropZone::Left | DropZone::Right } if group == source_group)
        }));
        assert!(zones.iter().any(|zone| {
            zone.zone == DropZone::ColumnLeft
                && matches!(zone.target, DropTarget::MainColumn { .. } | DropTarget::GroupEdge { zone: DropZone::Left, .. })
        }));
        assert_ne!(source_float, crate::FloatingGroupId::null());

        let before = format!("{layout:?}");
        let version = layout.version();
        assert!(layout.apply(DockCommand::SplitGroup {
            group: main_group,
            target_group: source_group,
            zone: DropZone::Left,
            ratio: 0.5,
        }, &metrics).is_err());
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
        let source = DropSource::Group { group: source_group };
        let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
        let center = zones.iter().find(|zone| {
            zone.target == DropTarget::GroupTabs { group: target_group, index: 1 }
                && zone.zone == DropZone::Center
        }).expect("target group's center should accept a group merge");
        let point = Point::new(
            center.rect.origin.x + center.rect.size.width / 2.0,
            center.rect.origin.y + center.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.resolve_drop(point, source, &rects, &metrics).unwrap(),
            Some(DropTarget::GroupTabs { group: target_group, index: 1 })
        );

        let before_version = layout.version();
        layout.apply(
            DockCommand::MergeGroup {
                group: source_group,
                target_group,
                index: 1,
            },
            &metrics,
        ).unwrap();
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
}
