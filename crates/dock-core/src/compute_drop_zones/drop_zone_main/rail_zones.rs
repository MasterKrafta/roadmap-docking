use super::*;

impl Layout {
    pub(in crate::compute_drop_zones) fn append_rail_insertion_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source_panels,
            tree_index,
            rects,
            left_columns,
            right_columns,
            ..
        } = *context;
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
