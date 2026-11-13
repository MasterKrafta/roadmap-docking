use super::*;

/// Main defines the canonical tab-strip behavior. Floating compositions reuse
/// these same append-only rules and full Group-header hit rectangles.
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

        // A dragged Panel appends to the end of any eligible Main or Floating
        // tab strip. It does not expose arbitrary header insertion slots.
        if matches!(source, DropSource::Panel { .. }) {
            for strip in &rects.tab_strips {
                let Some(area) = target_area(self, tree_index, strip.group) else {
                    continue;
                };
                if area == AllowedArea::ICON_RAIL || !source_allows(self, source_panels, area) {
                    continue;
                }
                let Some(Node::Tabs(tabs)) = self.node(strip.group) else {
                    continue;
                };
                let Some(group_rect) = rects
                    .groups
                    .iter()
                    .find(|group| group.group == strip.group)
                    .map(|group| group.rect)
                else {
                    continue;
                };
                push_zone(
                    zones,
                    DropTarget::Tabs {
                        group: strip.group,
                        index: tabs.children.len(),
                    },
                    DropZone::TabInsertion,
                    super::tab_strip_drop_rect(group_rect, strip.rect)?,
                )?;
            }
        }

        // A normal Group drag and the strict Photoshop 1/1/1 composition both
        // append the entire Tabs group to a target tab strip. Other compositions
        // retain their composition-level destinations and do not merge here.
        let simple_composition = matches!(source, DropSource::Composition { .. })
            && self.is_simple_composition_root(source_root_id);
        if !matches!(source, DropSource::Group { .. }) && !simple_composition {
            return Ok(());
        }
        let source_group = match source {
            DropSource::Group { .. } => source_root_id,
            DropSource::Composition { .. } if simple_composition => self
                .simple_composition_group(source_root_id)
                .ok_or(ComputeRectsError::InvalidLayoutState)?,
            _ => return Ok(()),
        };

        if self.profile() == Profile::Photoshop {
            for strip in &rects.tab_strips {
                let Some(area) = target_area(self, tree_index, strip.group) else {
                    continue;
                };
                if !matches!(area, AllowedArea::MAIN_DOCK | AllowedArea::FLOATING)
                    || !source_allows(self, source_panels, area)
                    || contains_node(tree_index, source_group, strip.group)
                    || contains_node(tree_index, strip.group, source_group)
                {
                    continue;
                }
                let Some(Node::Tabs(tabs)) = self.node(strip.group) else {
                    continue;
                };
                let Some(group_rect) = rects
                    .groups
                    .iter()
                    .find(|group| group.group == strip.group)
                    .map(|group| group.rect)
                else {
                    continue;
                };
                push_zone(
                    zones,
                    DropTarget::GroupTabs {
                        group: strip.group,
                        index: tabs.children.len(),
                    },
                    DropZone::TabInsertion,
                    super::tab_strip_drop_rect(group_rect, strip.rect)?,
                )?;
            }
        }

        for header in &rects.tab_headers {
            let Some(area) = target_area(self, tree_index, header.group) else {
                continue;
            };
            if area == AllowedArea::ICON_RAIL || !source_allows(self, source_panels, area) {
                continue;
            }
            // Photoshop Main and Floating group drops use the full append-only
            // strip above rather than per-tab insertion halves.
            if self.profile() == Profile::Photoshop
                && (area == AllowedArea::MAIN_DOCK || area == AllowedArea::FLOATING)
            {
                continue;
            }
            if contains_node(tree_index, source_group, header.group)
                || contains_node(tree_index, header.group, source_group)
            {
                continue;
            }
            let half = header.rect.size.width / 2.0;
            let left_rect = Rect::new(header.rect.origin, Size::new(half, header.rect.size.height));
            let right_rect = Rect::new(
                Point::new(header.rect.origin.x + half, header.rect.origin.y),
                Size::new(header.rect.size.width - half, header.rect.size.height),
            );
            push_zone(
                zones,
                DropTarget::GroupTabs {
                    group: header.group,
                    index: header.index,
                },
                DropZone::TabInsertion,
                left_rect,
            )?;
            push_zone(
                zones,
                DropTarget::GroupTabs {
                    group: header.group,
                    index: header.index + 1,
                },
                DropZone::TabInsertion,
                right_rect,
            )?;
        }
        Ok(())
    }
}


/// Adds the same semantic center target used by Main and Floating Group surfaces.
pub(super) fn append_group_center_zone(
    layout: &Layout,
    source: DropSource,
    source_root: NodeId,
    source_panels: &[PanelId],
    tree_index: &DropZoneTreeIndex,
    group_rect: crate::GroupRect,
    area: AllowedArea,
    metrics: &DockMetrics,
    zone_kind: DropZone,
    zones: &mut Vec<DropZoneRect>,
) -> Result<(), ComputeRectsError> {
    let Some(Node::Tabs(tabs)) = layout.node(group_rect.group) else {
        return Ok(());
    };

    let source_contains_target = matches!(
        source,
        DropSource::Group { .. } | DropSource::Composition { .. }
    ) && contains_node(tree_index, source_root, group_rect.group);
    let source_panel_is_here = match source {
        DropSource::Panel { panel } => {
            panel_node(tree_index, panel).and_then(|node| direct_parent(tree_index, node))
                == Some(group_rect.group)
        }
        DropSource::Group { .. } | DropSource::Composition { .. } => false,
    };
    let simple_composition = matches!(source, DropSource::Composition { .. })
        && layout.is_simple_composition_root(source_root);

    if source_contains_target
        || source_panel_is_here
        || (matches!(source, DropSource::Composition { .. }) && !simple_composition)
        || !source_allows(layout, source_panels, area)
    {
        return Ok(());
    }

    let target = match source {
        DropSource::Panel { .. } => DropTarget::Tabs {
            group: group_rect.group,
            index: tabs.children.len(),
        },
        DropSource::Group { .. } => DropTarget::GroupTabs {
            group: group_rect.group,
            index: tabs.children.len(),
        },
        DropSource::Composition { .. } if simple_composition => DropTarget::GroupTabs {
            group: group_rect.group,
            index: tabs.children.len(),
        },
        DropSource::Composition { .. } => return Ok(()),
    };
    push_zone(
        zones,
        target,
        zone_kind,
        group_center_rect(group_rect.rect, metrics)?,
    )
}


pub(super) fn shared_group_vertical_edge_rect(
    layout: &Layout,
    rects: &LayoutRects,
    tree_index: &DropZoneTreeIndex,
    group: NodeId,
    group_rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Option<Rect>, ComputeRectsError> {
    if layout.profile() != Profile::Photoshop {
        return Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?));
    }

    let Some(column_id) = owner_column(layout, tree_index, group) else {
        return Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?));
    };
    let Some(Node::Column(column)) = layout.node(column_id) else {
        return Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?));
    };
    let Some(group_index) = column.groups.iter().position(|candidate| *candidate == group) else {
        return Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?));
    };

    match edge {
        // The previous Group owns the splitter-aligned Bottom target. Keep
        // this Group's own Top header edge available as a separate target.
        DropZone::Top if group_index > 0 => {
            Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?))
        }
        DropZone::Top => {
            let Some(column_rect) = rects
                .columns
                .iter()
                .find(|column_rect| column_rect.column == column_id)
                .map(|column_rect| column_rect.rect)
            else {
                return Ok(Some(group_edge_rect(
                    group_rect,
                    edge,
                    metrics.edge_drop_width,
                )?));
            };
            let header_height = (group_rect.origin.y - column_rect.origin.y).max(0.0);
            let height = metrics.edge_drop_width.min(header_height) * 0.5;
            if height <= 0.0 {
                return Ok(Some(group_edge_rect(
                    group_rect,
                    edge,
                    metrics.edge_drop_width,
                )?));
            }
            let header_band = Rect::new(
                Point::new(group_rect.origin.x, group_rect.origin.y - height),
                Size::new(group_rect.size.width, height),
            );
            header_band.validate().map_err(ComputeRectsError::Geometry)?;
            Ok(Some(header_band))
        }
        DropZone::Bottom if group_index + 1 < column.groups.len() => {
            let Some(splitter) = rects.splitters.iter().find(|splitter| {
                splitter.owner == column_id
                    && splitter.axis == crate::Axis::Y
                    && splitter.boundary_index == group_index
            }) else {
                return Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?));
            };
            let height = metrics.edge_drop_width.max(splitter.rect.size.height);
            let center_y = splitter.rect.origin.y + splitter.rect.size.height * 0.5;
            let boundary = Rect::new(
                Point::new(group_rect.origin.x, center_y - height * 0.5),
                Size::new(group_rect.size.width, height),
            );
            boundary.validate().map_err(ComputeRectsError::Geometry)?;
            Ok(Some(boundary))
        }
        DropZone::Bottom => {
            Ok(Some(group_edge_rect(group_rect, edge, metrics.edge_drop_width)?))
        }
        _ => Err(ComputeRectsError::InvalidLayoutState),
    }
}
