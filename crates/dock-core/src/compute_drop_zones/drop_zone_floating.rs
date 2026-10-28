use super::*;
use crate::{FloatingGroupRect, GroupRect};

struct FloatingZoneContext<'a> {
    layout: &'a Layout,
    source: DropSource,
    source_root: NodeId,
    source_panels: &'a [PanelId],
    tree_index: &'a DropZoneTreeIndex,
    rects: &'a LayoutRects,
    metrics: &'a DockMetrics,
    zones: &'a mut Vec<DropZoneRect>,
}

impl Layout {
    pub(in crate::compute_drop_zones) fn append_floating_window_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        FloatingZoneContext {
            layout: self,
            source: context.source,
            source_root: context.source_root,
            source_panels: context.source_panels,
            tree_index: context.tree_index,
            rects: context.rects,
            metrics: context.metrics,
            zones,
        }
        .append_all()
    }
}

impl FloatingZoneContext<'_> {
    fn append_all(&mut self) -> Result<(), ComputeRectsError> {
        // Floating-window drop zones are stacked over their contents. Their outer
        // top/bottom targets address the first/last group in a vertical stack.
        for index in (0..self.rects.floating_groups.len()).rev() {
            let floating_rect = self.rects.floating_groups[index];
            self.append_floating_group(floating_rect)?;
        }
        Ok(())
    }

    fn append_floating_group(
        &mut self,
        floating_rect: FloatingGroupRect,
    ) -> Result<(), ComputeRectsError> {
        let Some(floating) = self.layout.floating_group(floating_rect.group) else {
            return Ok(());
        };
        let Some(area) = self.layout.dock_area(floating.area) else {
            return Ok(());
        };
        if !source_allows(self.layout, self.source_panels, AllowedArea::FLOATING) {
            return Ok(());
        }

        let mut target_groups: Vec<GroupRect> = self
            .rects
            .groups
            .iter()
            .copied()
            .filter(|group| contains_node(self.tree_index, area.root, group.group))
            .collect();
        target_groups.sort_by(|left, right| {
            left.rect
                .origin
                .y
                .partial_cmp(&right.rect.origin.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if target_groups.is_empty() {
            return Ok(());
        }

        self.append_center_zones(&target_groups)?;
        self.append_edge_zones(floating_rect, area.root, &target_groups)?;
        if self.layout.profile() == Profile::Generic {
            self.append_side_zones(&target_groups)?;
        }
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    pub(super) fn append_center_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // Multiple groups can share a vertical position in a horizontal floating
        // composition, so center zones are emitted per group.
        for target_group in target_groups {
            let tabs = match self.layout.node(target_group.group) {
                Some(Node::Tabs(tabs)) if !tabs.collapsed => tabs,
                _ => continue,
            };
            let is_self = matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group);
            let panel_already_in_group = match self.source {
                DropSource::Panel { panel } => {
                    panel_node(self.tree_index, panel)
                        .and_then(|node_id| direct_parent(self.tree_index, node_id))
                        == Some(target_group.group)
                }
                DropSource::Group { .. } => false,
            };
            if is_self || panel_already_in_group {
                continue;
            }

            let target = match self.source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
            };
            push_zone(
                self.zones,
                target,
                DropZone::FloatingCenter,
                group_center_rect(target_group.rect, self.metrics)?,
            )?;
        }
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    pub(super) fn append_side_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // The Generic profile supports horizontal nested splits within a
        // floating area; Photoshop uses its column-edge policy instead.
        for target_group in target_groups {
            if matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Left, DropZone::Left),
                (DropZone::Right, DropZone::Right),
            ] {
                if group_edge_is_noop(
                    self.layout,
                    self.tree_index,
                    self.source,
                    target_group.group,
                    edge,
                ) {
                    continue;
                }
                push_zone(
                    self.zones,
                    DropTarget::GroupEdge {
                        group: target_group.group,
                        zone: edge,
                    },
                    kind,
                    group_edge_rect(target_group.rect, edge, self.metrics.edge_drop_width)?,
                )?;
            }
        }
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    fn append_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        root: NodeId,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        let top_rect = floating_edge_rect(floating_rect.rect, DropZone::Top, self.metrics)?;
        let bottom_rect = floating_edge_rect(floating_rect.rect, DropZone::Bottom, self.metrics)?;
        let horizontal_composition = matches!(
            self.layout.node(root),
            Some(Node::Split(split)) if split.axis == crate::Axis::X
        );

        if horizontal_composition {
            self.append_horizontal_edge_zones(floating_rect, target_groups)?;
        } else {
            self.append_vertical_edge_zones(top_rect, bottom_rect, target_groups)?;
        }
        Ok(())
    }

    fn append_horizontal_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // In a horizontal composition, top/bottom edge zones span each column
        // independently, so a drop over one branch targets that branch.
        let mut columns = Vec::new();
        for target_group in target_groups {
            let Some(column) = owner_column(self.layout, self.tree_index, target_group.group)
            else {
                continue;
            };
            if !columns.contains(&column) {
                columns.push(column);
            }
        }
        for column in columns {
            self.append_column_edge_zones(floating_rect, column, target_groups)?;
        }

        // Generic-profile horizontal splits may contain Tabs nodes directly,
        // without a containing Column. Clip their edge zones to each group.
        for target_group in target_groups
            .iter()
            .filter(|group| owner_column(self.layout, self.tree_index, group.group).is_none())
        {
            self.append_direct_branch_edge_zones(floating_rect.rect, *target_group)?;
        }
        Ok(())
    }

    fn append_column_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        column: NodeId,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        let mut column_groups: Vec<GroupRect> = target_groups
            .iter()
            .copied()
            .filter(|group| owner_column(self.layout, self.tree_index, group.group) == Some(column))
            .collect();
        column_groups.sort_by(|left, right| {
            left.rect
                .origin
                .y
                .partial_cmp(&right.rect.origin.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some(first_group) = column_groups.first().copied() else {
            return Ok(());
        };
        let Some(last_group) = column_groups.last().copied() else {
            return Ok(());
        };
        let Some(column_rect) = self
            .rects
            .columns
            .iter()
            .find(|entry| entry.column == column)
        else {
            return Ok(());
        };

        let column_top_rect = floating_edge_rect_for_branch(
            floating_rect.rect,
            column_rect.rect,
            DropZone::Top,
            self.metrics,
        )?;
        let column_bottom_rect = floating_edge_rect_for_branch(
            floating_rect.rect,
            column_rect.rect,
            DropZone::Bottom,
            self.metrics,
        )?;
        self.push_floating_edge_zone(
            first_group.group,
            DropZone::Top,
            DropZone::FloatingTop,
            column_top_rect,
        )?;
        self.push_floating_edge_zone(
            last_group.group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
            column_bottom_rect,
        )?;
        Ok(())
    }

    fn append_direct_branch_edge_zones(
        &mut self,
        floating_bounds: Rect,
        target_group: GroupRect,
    ) -> Result<(), ComputeRectsError> {
        let top_branch_rect = floating_edge_rect_for_branch(
            floating_bounds,
            target_group.rect,
            DropZone::Top,
            self.metrics,
        )?;
        let bottom_branch_rect = floating_edge_rect_for_branch(
            floating_bounds,
            target_group.rect,
            DropZone::Bottom,
            self.metrics,
        )?;
        self.push_floating_edge_zone(
            target_group.group,
            DropZone::Top,
            DropZone::FloatingTop,
            top_branch_rect,
        )?;
        self.push_floating_edge_zone(
            target_group.group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
            bottom_branch_rect,
        )?;
        Ok(())
    }

    fn append_vertical_edge_zones(
        &mut self,
        top_rect: Rect,
        bottom_rect: Rect,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // A vertically stacked floating root has one outer top and bottom,
        // targeting its first and last groups respectively.
        let Some(first_group) = target_groups.first().copied() else {
            return Ok(());
        };
        let Some(last_group) = target_groups.last().copied() else {
            return Ok(());
        };
        self.push_floating_edge_zone(
            first_group.group,
            DropZone::Top,
            DropZone::FloatingTop,
            top_rect,
        )?;
        self.push_floating_edge_zone(
            last_group.group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
            bottom_rect,
        )?;
        Ok(())
    }

    fn push_floating_edge_zone(
        &mut self,
        target_group: NodeId,
        edge: DropZone,
        kind: DropZone,
        rect: Rect,
    ) -> Result<(), ComputeRectsError> {
        if matches!(self.source, DropSource::Group { .. })
            && group_edge_is_noop(
                self.layout,
                self.tree_index,
                self.source,
                target_group,
                edge,
            )
        {
            return Ok(());
        }
        push_zone(
            self.zones,
            DropTarget::GroupEdge {
                group: target_group,
                zone: edge,
            },
            kind,
            rect,
        )
    }
}
