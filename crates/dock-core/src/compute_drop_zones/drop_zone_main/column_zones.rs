use super::*;

impl Layout {
    pub(in crate::compute_drop_zones) fn append_main_column_zones(
        &self,
        source: DropSource,
        source_panels: &[PanelId],
        tree_index: &DropZoneTreeIndex,
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
}
