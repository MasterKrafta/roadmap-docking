use super::*;

impl Layout {
    pub(super) fn append_main_group_zones(
        &self,
        source: DropSource,
                source_root: NodeId,
                source_panels: &[PanelId],
                tree_index: &DropZoneTreeIndex,
                rects: &LayoutRects,
                metrics: &DockMetrics,
                left_columns: &[(NodeId, Rect)],
                right_columns: &[(NodeId, Rect)],
                may_add_column: bool,
                zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
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
            };
            if !source_contains_target
                && !source_panel_is_here
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
                push_zone(zones, target, DropZone::Center, center_rect)?;
            }

            if source_contains_target
                || (matches!(source, DropSource::Group { .. })
                    && contains_node(tree_index, group_rect.group, source_root))
                || !source_allows(self, &source_panels, area)
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
            if let Some((side, column_index, _column_rect)) =
                group_main_column(self, tree_index, group_rect.group, &left_columns, &right_columns)
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
                    if !group_edge_is_noop(self, tree_index, source, group_rect.group, DropZone::Left) {
                        push_zone(zones, left_target, DropZone::ColumnLeft, left_edge)?;
                    }
                    if !group_edge_is_noop(self, tree_index, source, group_rect.group, DropZone::Right) {
                        push_zone(zones, right_target, DropZone::ColumnRight, right_edge)?;
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

    pub(super) fn append_tab_insertion_zones(
        &self,
        source: DropSource,
                source_panels: &[PanelId],
                tree_index: &DropZoneTreeIndex,
                rects: &LayoutRects,
                zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        // Tab strips are independent insertion/reorder affordances. Each header
        // is split into two half-open halves mapping to the exact slots before/after it.
        for header in &rects.tab_headers {
            let Some(area) = target_area(self, tree_index, header.group) else {
                continue;
            };
            if area == AllowedArea::ICON_RAIL || !source_allows(self, &source_panels, area) {
                continue;
            }
            if matches!(source, DropSource::Group { .. })
                && (contains_node(tree_index, source_root, header.group)
                    || contains_node(tree_index, header.group, source_root))
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
            };
            push_zone(zones, right_target, DropZone::TabInsertion, right_rect)?;
        }
        Ok(())
    }

    pub(super) fn append_main_column_zones(
        &self,
        source: DropSource,
                source_panels: &[PanelId],
                tree_index: &DropZoneTreeIndex,
                rects: &LayoutRects,
                metrics: &DockMetrics,
                left_columns: &[(NodeId, Rect)],
                right_columns: &[(NodeId, Rect)],
                may_add_column: bool,
                zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
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
                if !matches!(
                    source,
                    DropSource::Group { group }
                        if self.group_slot_is_noop(group, *column_id, end_index)
                ) {
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
                let left_rect = Rect::new(
                    column_rect.origin,
                    Size::new(edge_width, column_rect.size.height),
                );
                let right_rect = Rect::new(
                    Point::new(
                        column_rect.origin.x + column_rect.size.width - edge_width,
                        column_rect.origin.y,
                    ),
                    Size::new(edge_width, column_rect.size.height),
                );
                if may_add_column {
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

    pub(super) fn append_rail_insertion_zones(
        &self,
        source: DropSource,
                source_panels: &[PanelId],
                tree_index: &DropZoneTreeIndex,
                rects: &LayoutRects,
                left_columns: &[(NodeId, Rect)],
                right_columns: &[(NodeId, Rect)],
                zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
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
