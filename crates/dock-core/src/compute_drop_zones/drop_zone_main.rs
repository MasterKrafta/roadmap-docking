use super::*;

impl Layout {
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
        // Main-dock groups expose semantic center, vertical and horizontal targets.
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

            append_group_center_zone(
                self,
                source,
                source_root,
                source_panels,
                tree_index,
                *group_rect,
                area,
                metrics,
                DropZone::Center,
                zones,
            )?;
            let source_contains_target = matches!(
                source,
                DropSource::Group { .. } | DropSource::Composition { .. }
            ) && contains_node(tree_index, source_root, group_rect.group);

            if source_contains_target
                || (matches!(source, DropSource::Group { .. })
                    && contains_node(tree_index, group_rect.group, source_root))
                || !source_allows(self, source_panels, area)
            {
                continue;
            }

            // Whole Floating compositions can be inserted above or below any
            // eligible Main Group, including a Group in a single-Group Column.
            // Splitter-aligned internal boundaries are represented by the
            // preceding Group's Bottom target. Each Group also exposes its own Top edge.

            for (edge, kind) in [
                (DropZone::Top, DropZone::Top),
                (DropZone::Bottom, DropZone::Bottom),
            ] {
                let zone_rect = shared_group_vertical_edge_rect(
                    self,
                    rects,
                    tree_index,
                    group_rect.group,
                    group_rect.rect,
                    edge,
                    metrics,
                )?;
                let Some(zone_rect) = zone_rect else {
                    continue;
                };
                if group_edge_is_noop(self, tree_index, source, group_rect.group, edge) {
                    continue;
                }
                let target = DropTarget::GroupEdge {
                    group: group_rect.group,
                    zone: edge,
                };
                push_zone(zones, target, kind, zone_rect)?;
            }

            // Photoshop exposes horizontal insertion at the Column boundary,
            // not on each individual Group. Its full-height ColumnLeft /
            // ColumnRight targets are built by append_main_column_zones.
            // Generic layouts retain group-scoped horizontal splitting.
            if self.profile() == Profile::Generic {
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
                            DropSource::Composition { .. } => DropTarget::MainColumn {
                                side,
                                index: column_index,
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
                            DropSource::Composition { .. } => DropTarget::MainColumn {
                                side,
                                index: column_index + 1,
                            },
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
                } else {
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

/// Returns the vertical edge band for a Main group.
///
/// On Photoshop layouts, the boundary between two stacked groups has one
/// splitter-aligned drop strip represented by the upper Group's Bottom edge.
/// The lower Group still exposes its own Top header edge as a separate target.
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
                if !matches!(source, DropSource::Composition { .. })
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
                        DropSource::Composition { .. } => Some((
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
            source_root,
            source_panels,
            tree_index,
            rects,
            left_columns,
            right_columns,
            ..
        } = *context;
        let simple_composition = matches!(source, DropSource::Composition { .. })
            && self.is_simple_composition_root(source_root);
        if matches!(source, DropSource::Composition { .. }) && !simple_composition {
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
