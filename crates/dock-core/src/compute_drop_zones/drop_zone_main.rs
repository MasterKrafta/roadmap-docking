use super::*;

impl Layout {
    /// A Column can only move as a complete composition: to another Main Dock
    /// insertion slot or into free Canvas space as a floating root. It cannot
    /// merge into an individual Tabs group or become a child of another Column.
    pub(in crate::compute_drop_zones) fn append_column_drag_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_panels,
            rects,
            metrics,
            viewport,
            left_columns,
            right_columns,
            may_add_left_column,
            may_add_right_column,
            ..
        } = *context;
        let DropSource::Column { column: source_column } = source else {
            return Ok(());
        };

        let source_position = left_columns
            .iter()
            .position(|(id, _)| *id == source_column)
            .map(|index| (DockSide::Left, index))
            .or_else(|| {
                right_columns
                    .iter()
                    .position(|(id, _)| *id == source_column)
                    .map(|index| (DockSide::Right, index))
            });
        if source_position.is_none() && !source_is_floating_root(self, source_column) {
            return Err(ComputeRectsError::InvalidLayoutState);
        }

        let can_insert = |side| match side {
            DockSide::Left => may_add_left_column,
            DockSide::Right => may_add_right_column,
        };
        let is_same_position = |side, index| {
            source_position.is_some_and(|(source_side, source_index)| {
                source_side == side && (index == source_index || index == source_index + 1)
            })
        };

        if source_allows(self, source_panels, AllowedArea::MAIN_DOCK) {
            for (side, columns) in [
                (DockSide::Left, left_columns),
                (DockSide::Right, right_columns),
            ] {
                if !can_insert(side) {
                    continue;
                }
                for (index, (_, column_rect)) in columns.iter().enumerate() {
                    let edge_width = metrics.edge_drop_width.min(column_rect.size.width / 3.0);
                    let left_fallback = Rect::new(
                        column_rect.origin,
                        Size::new(edge_width, column_rect.size.height),
                    );
                    let right_fallback = Rect::new(
                        Point::new(
                            column_rect.origin.x + column_rect.size.width - edge_width,
                            column_rect.origin.y,
                        ),
                        Size::new(edge_width, column_rect.size.height),
                    );
                    let left_rect = main_column_boundary_drop_rect(
                        self, rects, side, columns, index, metrics,
                    )?
                    .unwrap_or(left_fallback);
                    let right_rect = main_column_boundary_drop_rect(
                        self, rects, side, columns, index + 1, metrics,
                    )?
                    .unwrap_or(right_fallback);

                    if !is_same_position(side, index) {
                        push_zone(
                            zones,
                            DropTarget::MainColumn { side, index },
                            DropZone::ColumnLeft,
                            left_rect,
                        )?;
                    }
                    if !is_same_position(side, index + 1) {
                        push_zone(
                            zones,
                            DropTarget::MainColumn {
                                side,
                                index: index + 1,
                            },
                            DropZone::ColumnRight,
                            right_rect,
                        )?;
                    }
                }
            }

            // The outer window edges also accept a whole Column, including when
            // the destination Main Dock is currently empty.
            if rects.canvas.is_some() {
                let edge_width = metrics.edge_drop_width.min(viewport.size.width / 3.0);
                for (side, columns, is_left) in [
                    (DockSide::Left, left_columns, true),
                    (DockSide::Right, right_columns, false),
                ] {
                    if !can_insert(side) {
                        continue;
                    }
                    let index = if is_left { 0 } else { columns.len() };
                    if is_same_position(side, index) {
                        continue;
                    }
                    let edge_rect = if is_left {
                        Rect::new(
                            viewport.origin,
                            Size::new(edge_width, viewport.size.height),
                        )
                    } else {
                        Rect::new(
                            Point::new(
                                viewport.origin.x + viewport.size.width - edge_width,
                                viewport.origin.y,
                            ),
                            Size::new(edge_width, viewport.size.height),
                        )
                    };
                    push_zone(
                        zones,
                        DropTarget::MainColumn { side, index },
                        DropZone::DockEdge,
                        edge_rect,
                    )?;
                }
            }
        }

        if source_allows(self, source_panels, AllowedArea::FLOATING)
            && let Some(canvas) = rects.canvas
        {
            push_zone(zones, DropTarget::FreeArea, DropZone::FreeArea, canvas)?;
        }
        Ok(())
    }

    pub(in crate::compute_drop_zones) fn append_main_group_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_root,
            source_panels,
            tree_index,
            rects,
            metrics,
            left_columns,
            right_columns,
            may_add_left_column,
            may_add_right_column,
            ..
        } = *context;
        let may_add_column = |side| match side {
            DockSide::Left => may_add_left_column,
            DockSide::Right => may_add_right_column,
        };
        // Main-dock groups: center/tab insertion, vertical group splitting,
        // and profile-correct horizontal/column splitting.
        for group_rect in &rects.groups {
            if !matches!(self.node(group_rect.group), Some(Node::Tabs(_))) {
                continue;
            }
            if is_rail_group(self, tree_index, group_rect.group) {
                continue;
            }
            let Some(area) = target_area(self, tree_index, group_rect.group) else {
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
                && contains_node(tree_index, source_root, group_rect.group);
            let source_panel_is_here = match source {
                DropSource::Panel { panel } => {
                    panel_node(tree_index, panel).and_then(|node| direct_parent(tree_index, node))
                        == Some(group_rect.group)
                }
                DropSource::Group { .. } => false,
                DropSource::Column { .. } => false,
            };
            if !source_contains_target
                && !source_panel_is_here
                && source_allows(self, source_panels, area)
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
                    DropSource::Column { .. } => continue,
                };
                push_zone(zones, target, DropZone::Center, center_rect)?;
            }

            if source_contains_target
                || (matches!(source, DropSource::Group { .. })
                    && contains_node(tree_index, group_rect.group, source_root))
                || !source_allows(self, source_panels, area)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Top, DropZone::Top),
                (DropZone::Bottom, DropZone::Bottom),
            ] {
                if group_edge_is_noop(self, tree_index, source, group_rect.group, edge) {
                    continue;
                }
                let target = DropTarget::GroupEdge {
                    group: group_rect.group,
                    zone: edge,
                };
                push_zone(
                    zones,
                    target,
                    kind,
                    group_edge_rect(group_rect.rect, edge, metrics.edge_drop_width)?,
                )?;
            }

            let left_edge =
                group_edge_rect(group_rect.rect, DropZone::Left, metrics.edge_drop_width)?;
            let right_edge =
                group_edge_rect(group_rect.rect, DropZone::Right, metrics.edge_drop_width)?;
            if let Some((side, column_index, _column_rect)) = group_main_column(
                self,
                tree_index,
                group_rect.group,
                left_columns,
                right_columns,
            ) {
                if may_add_column(side) {
                    let columns = match side {
                        DockSide::Left => left_columns,
                        DockSide::Right => right_columns,
                    };
                    let left_zone_rect = main_column_boundary_drop_rect(
                        self, rects, side, columns, column_index, metrics,
                    )?
                    .map(|boundary| preserve_vertical_span(boundary, left_edge))
                    .unwrap_or(left_edge);
                    let right_zone_rect = main_column_boundary_drop_rect(
                        self, rects, side, columns, column_index + 1, metrics,
                    )?
                    .map(|boundary| preserve_vertical_span(boundary, right_edge))
                    .unwrap_or(right_edge);
                    let left_target = match source {
                        DropSource::Panel { .. } => DropTarget::MainColumn {
                            side,
                            index: column_index,
                        },
                        DropSource::Group { .. } => DropTarget::GroupEdge {
                            group: group_rect.group,
                            zone: DropZone::Left,
                        },
                        DropSource::Column { .. } => continue,
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
                        DropSource::Column { .. } => continue,
                    };
                    if !group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        group_rect.group,
                        DropZone::Left,
                    ) {
                        push_zone(zones, left_target, DropZone::ColumnLeft, left_zone_rect)?;
                    }
                    if !group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        group_rect.group,
                        DropZone::Right,
                    ) {
                        push_zone(zones, right_target, DropZone::ColumnRight, right_zone_rect)?;
                    }
                }
            } else if self.profile() == Profile::Generic {
                // Generic layouts permit a nested horizontal Split around a group.
                for (edge, rect) in [(DropZone::Left, left_edge), (DropZone::Right, right_edge)] {
                    if !group_edge_is_noop(self, tree_index, source, group_rect.group, edge) {
                        push_zone(
                            zones,
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
        Ok(())
    }
}

impl Layout {
    pub(in crate::compute_drop_zones) fn append_tab_insertion_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_root: source_root_id,
            source_panels,
            tree_index,
            rects,
            ..
        } = *context;
        // A panel dropped on a Main Dock tab strip always appends to the
        // target group. Emit one zone for the whole strip, not one insertion
        // slot per existing tab header.
        if matches!(source, DropSource::Panel { .. }) {
            for strip in &rects.tab_strips {
                if target_area(self, tree_index, strip.group) != Some(AllowedArea::MAIN_DOCK) {
                    continue;
                }
                let Some(Node::Tabs(tabs)) = self.node(strip.group) else {
                    continue;
                };
                push_zone(
                    zones,
                    DropTarget::Tabs {
                        group: strip.group,
                        index: tabs.children.len(),
                    },
                    DropZone::TabInsertion,
                    strip.rect,
                )?;
            }
        }

        // Whole Group drags retain exact insertion positions. Floating panel
        // insertion also retains its existing header-based behavior until Stage 5.
        for header in &rects.tab_headers {
            let Some(area) = target_area(self, tree_index, header.group) else {
                continue;
            };
            if area == AllowedArea::ICON_RAIL || !source_allows(self, source_panels, area) {
                continue;
            }
            // In Main Dock a panel tab always appends through the group's Center
            // target. Keep exact strip insertion slots for whole Group drags;
            // Floating panel insertion is handled in the Floating stage.
            if matches!(source, DropSource::Panel { .. }) && area != AllowedArea::FLOATING {
                continue;
            }
            if matches!(source, DropSource::Group { .. })
                && (contains_node(tree_index, source_root_id, header.group)
                    || contains_node(tree_index, header.group, source_root_id))
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
                DropSource::Column { .. } => continue,
            };
            push_zone(zones, target, DropZone::TabInsertion, left_rect)?;
            let right_target = match source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: header.group,
                    index: header.index + 1,
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: header.group,
                    index: header.index + 1,
                },
                DropSource::Column { .. } => continue,
            };
            push_zone(zones, right_target, DropZone::TabInsertion, right_rect)?;
        }
        Ok(())
    }
}

/// Returns a drop strip aligned with one insertion boundary in a main Dock.
///
/// Interior boundaries use the Split(X) hit geometry. The Canvas-facing outer
/// boundary uses the owning Column's sentinel splitter and shifts slightly into
/// the free Canvas side. The window-facing boundary is left to DockEdge.
fn main_column_boundary_drop_rect(
    layout: &Layout,
    rects: &LayoutRects,
    side: DockSide,
    columns: &[(NodeId, Rect)],
    index: usize,
    metrics: &DockMetrics,
) -> Result<Option<Rect>, ComputeRectsError> {
    if columns.is_empty() || index > columns.len() {
        return Ok(None);
    }

    let is_window_edge = match side {
        DockSide::Left => index == 0,
        DockSide::Right => index == columns.len(),
    };
    if is_window_edge {
        return Ok(None);
    }

    let is_canvas_edge = match side {
        DockSide::Left => index == columns.len(),
        DockSide::Right => index == 0,
    };
    let splitter = if is_canvas_edge {
        let edge_column = match side {
            DockSide::Left => columns.last().map(|(id, _)| *id),
            DockSide::Right => columns.first().map(|(id, _)| *id),
        };
        edge_column.and_then(|column| {
            rects.splitters.iter().find(|splitter| {
                splitter.owner == column
                    && splitter.axis == crate::Axis::X
                    && splitter.boundary_index == crate::OUTER_COLUMN_EDGE_BOUNDARY_INDEX
            })
        })
    } else {
        let root = layout.main_area(side).map(|area| area.root);
        root.and_then(|root| {
            rects.splitters.iter().find(|splitter| {
                splitter.owner == root
                    && splitter.axis == crate::Axis::X
                    && splitter.boundary_index == index - 1
            })
        })
    };

    let Some(splitter) = splitter else {
        return Ok(None);
    };

    let width = metrics.edge_drop_width.max(splitter.rect.size.width);
    let mut center_x = splitter.rect.origin.x + splitter.rect.size.width * 0.5;
    if is_canvas_edge {
        let offset = metrics.splitter_thickness;
        center_x += match side {
            DockSide::Left => offset,
            DockSide::Right => -offset,
        };
    }

    let result = Rect::new(
        Point::new(center_x - width * 0.5, splitter.rect.origin.y),
        Size::new(width, splitter.rect.size.height),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(Some(result))
}

/// Uses the splitter-aligned horizontal band while preserving a group's own
/// vertical extent, so stacked groups keep independent drop targets.
fn preserve_vertical_span(boundary: Rect, group_edge: Rect) -> Rect {
    Rect::new(
        Point::new(boundary.origin.x, group_edge.origin.y),
        Size::new(boundary.size.width, group_edge.size.height),
    )
}

impl Layout {
    pub(in crate::compute_drop_zones) fn append_main_column_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_panels,
            tree_index,
            rects,
            metrics,
            left_columns,
            right_columns,
            may_add_left_column,
            may_add_right_column,
            ..
        } = *context;
        let may_add_column = |side| match side {
            DockSide::Left => may_add_left_column,
            DockSide::Right => may_add_right_column,
        };
        // Main-column side targets and append-to-column targets.
        for (side, columns) in [
            (DockSide::Left, left_columns),
            (DockSide::Right, right_columns),
        ] {
            for (index, (column_id, column_rect)) in columns.iter().enumerate() {
                let Some(Node::Column(column)) = self.node(*column_id) else {
                    continue;
                };
                if column.mode.is_rail() {
                    continue;
                }
                if !source_allows(self, source_panels, AllowedArea::MAIN_DOCK) {
                    continue;
                }
                let bottom = column_rect
                    .checked_bottom()
                    .map_err(ComputeRectsError::Geometry)?;
                let end_height = metrics.edge_drop_width.min(column_rect.size.height / 3.0);
                let side_inset = metrics.edge_drop_width.min(column_rect.size.width / 4.0);
                let end_rect = Rect::new(
                    Point::new(column_rect.origin.x + side_inset, bottom - end_height),
                    Size::new(
                        (column_rect.size.width - 2.0 * side_inset).max(0.0),
                        end_height,
                    ),
                );
                let end_index = column.groups.len();
                if !matches!(source, DropSource::Column { .. })
                    && !matches!(
                        source,
                        DropSource::Group { group }
                            if self.group_slot_is_noop(group, *column_id, end_index)
                    )
                {
                    push_zone(
                        zones,
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
                let left_fallback = Rect::new(
                    column_rect.origin,
                    Size::new(edge_width, column_rect.size.height),
                );
                let right_fallback = Rect::new(
                    Point::new(
                        column_rect.origin.x + column_rect.size.width - edge_width,
                        column_rect.origin.y,
                    ),
                    Size::new(edge_width, column_rect.size.height),
                );
                let left_rect = main_column_boundary_drop_rect(
                    self, rects, side, columns, index, metrics,
                )?
                .unwrap_or(left_fallback);
                let right_rect = main_column_boundary_drop_rect(
                    self, rects, side, columns, index + 1, metrics,
                )?
                .unwrap_or(right_fallback);
                if may_add_column(side) {
                    let targets = match source {
                        DropSource::Panel { .. } => Some((
                            DropTarget::MainColumn { side, index },
                            DropTarget::MainColumn {
                                side,
                                index: index + 1,
                            },
                        )),
                        DropSource::Group { .. } => edge_group_for_column(self, *column_id, false)
                            .zip(edge_group_for_column(self, *column_id, true))
                            .map(|(left_group, right_group)| {
                                (
                                    DropTarget::GroupEdge {
                                        group: left_group,
                                        zone: DropZone::Left,
                                    },
                                    DropTarget::GroupEdge {
                                        group: right_group,
                                        zone: DropZone::Right,
                                    },
                                )
                            }),
                        DropSource::Column { .. } => Some((
                            DropTarget::MainColumn { side, index },
                            DropTarget::MainColumn {
                                side,
                                index: index + 1,
                            },
                        )),
                    };
                    if let Some((left_target, right_target)) = targets {
                        let left_is_noop = match &left_target {
                            DropTarget::GroupEdge { group, zone } => {
                                group_edge_is_noop(self, tree_index, source, *group, *zone)
                            }
                            _ => false,
                        };
                        let right_is_noop = match &right_target {
                            DropTarget::GroupEdge { group, zone } => {
                                group_edge_is_noop(self, tree_index, source, *group, *zone)
                            }
                            _ => false,
                        };
                        if !left_is_noop {
                            push_zone(zones, left_target, DropZone::ColumnLeft, left_rect)?;
                        }
                        if !right_is_noop {
                            push_zone(zones, right_target, DropZone::ColumnRight, right_rect)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Layout {
    pub(in crate::compute_drop_zones) fn append_rail_insertion_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_panels,
            tree_index,
            rects,
            left_columns,
            right_columns,
            ..
        } = *context;
        if matches!(source, DropSource::Column { .. }) {
            return Ok(());
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
            if !source_allows(self, source_panels, AllowedArea::ICON_RAIL) {
                continue;
            }
            let mut icons: Vec<_> = rects
                .tab_headers
                .iter()
                .filter(|icon| {
                    column
                        .groups
                        .iter()
                        .any(|root| contains_node(tree_index, *root, icon.group))
                })
                .collect();
            icons.sort_by(|left, right| {
                left.rect
                    .origin
                    .y
                    .partial_cmp(&right.rect.origin.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if icons.is_empty() {
                push_zone(
                    zones,
                    DropTarget::RailSlot {
                        column: column_id,
                        index: 0,
                    },
                    DropZone::RailInsertion,
                    column_rect.rect,
                )?;
            } else {
                for (index, icon) in icons.iter().enumerate() {
                    let top_height = icon.rect.size.height / 2.0;
                    push_zone(
                        zones,
                        DropTarget::RailSlot {
                            column: column_id,
                            index,
                        },
                        DropZone::RailInsertion,
                        Rect::new(
                            icon.rect.origin,
                            Size::new(icon.rect.size.width, top_height),
                        ),
                    )?;
                    push_zone(
                        zones,
                        DropTarget::RailSlot {
                            column: column_id,
                            index: index + 1,
                        },
                        DropZone::RailInsertion,
                        Rect::new(
                            Point::new(icon.rect.origin.x, icon.rect.origin.y + top_height),
                            Size::new(icon.rect.size.width, icon.rect.size.height - top_height),
                        ),
                    )?;
                }
            }
        }
        Ok(())
    }
}
