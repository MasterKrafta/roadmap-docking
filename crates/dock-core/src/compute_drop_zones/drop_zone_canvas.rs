use super::*;

impl Layout {
    pub(super) fn append_canvas_edge_zones(
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
            viewport,
            left_columns,
            right_columns,
            may_add_column,
        } = *context;
        // The two outside viewport edges open/extend a dock. A whole group uses
        // its edge operation when a neighbouring column exists; panels use an exact slot.
        if source_allows(self, source_panels, AllowedArea::MAIN_DOCK) && may_add_column {
            let edge_width = metrics.edge_drop_width.min(viewport.size.width / 3.0);
            for (side, columns, zone, is_left) in [
                (DockSide::Left, left_columns, DropZone::DockEdge, true),
                (DockSide::Right, right_columns, DropZone::DockEdge, false),
            ] {
                let rect = if is_left {
                    Rect::new(viewport.origin, Size::new(edge_width, viewport.size.height))
                } else {
                    Rect::new(
                        Point::new(
                            viewport.origin.x + viewport.size.width - edge_width,
                            viewport.origin.y,
                        ),
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
                            zone: if is_left {
                                DropZone::Left
                            } else {
                                DropZone::Right
                            },
                        }),
                };
                if let Some(target) = target {
                    let is_noop = match target {
                        DropTarget::GroupEdge { group, zone } => {
                            group_edge_is_noop(self, tree_index, source, group, zone)
                        }
                        _ => false,
                    };
                    if !is_noop {
                        push_zone(zones, target, zone, rect)?;
                    }
                }
            }
        }

        // A free Canvas is a Float command hint, never a fake tab/column destination.
        if source_allows(self, source_panels, AllowedArea::FLOATING)
            && !source_is_floating_root(self, source_root)
            && let Some(canvas) = rects.canvas
        {
            push_zone(zones, DropTarget::FreeArea, DropZone::FreeArea, canvas)?;
        }
        Ok(())
    }
}
