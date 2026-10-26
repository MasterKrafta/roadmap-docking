use super::*;

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
        // Tab strips are independent insertion/reorder affordances. Each header
        // is split into two half-open halves mapping to the exact slots before/after it.
        for header in &rects.tab_headers {
            let Some(area) = target_area(self, tree_index, header.group) else {
                continue;
            };
            if area == AllowedArea::ICON_RAIL || !source_allows(self, source_panels, area) {
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
}
