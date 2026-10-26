use super::*;

impl FloatingZoneContext<'_> {
    pub(super) fn append_edge_zones(
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
        let Some(column_rect) = self.rects.columns.iter().find(|entry| entry.column == column)
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
