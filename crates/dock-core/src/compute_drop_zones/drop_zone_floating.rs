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
        // Composition and Group sources use the same target geometry. The source
        // remains in its original owner until the final command. Floating zones are
        // stacked over contents; outer top/bottom targets address first/last groups.
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
        self.append_group_edge_zones(&target_groups)?;
        self.append_column_slot_zones(area.root)?;
        self.append_edge_zones(floating_rect, area.root, &target_groups)?;
        // Photoshop whole-composition side docking targets the outer edges of a
        // compatible Floating composition. Generic keeps its flexible group edges.
        if self.layout.profile() == Profile::Generic
            || matches!(
                self.source,
                DropSource::Group { .. } | DropSource::Composition { .. }
            )
        {
            self.append_side_zones(&target_groups, area.root)?;
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
                DropSource::Group { .. } | DropSource::Composition { .. } => false,
            };
            if is_self || panel_already_in_group {
                continue;
            }

            let target = match self.source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
                DropSource::Group { .. }
                    if matches!(self.layout.node(self.source_root), Some(Node::Tabs(_))) =>
                {
                    DropTarget::GroupTabs {
                        group: target_group.group,
                        index: tabs.children.len(),
                    }
                }
                DropSource::Composition { .. } => continue,
                _ => continue,
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
    pub(super) fn append_group_edge_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        let source_can_split_vertically = match self.source {
            DropSource::Panel { .. } => true,
            DropSource::Group { .. } => {
                matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
            }
            DropSource::Composition { .. } => true,
        };
        if !source_can_split_vertically {
            return Ok(());
        }

        for target_group in target_groups {
            for edge in [DropZone::Top, DropZone::Bottom] {
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
                    edge,
                    floating_group_edge_rect(target_group.rect, edge, self.metrics)?,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn append_column_slot_zones(
        &mut self,
        floating_root: NodeId,
    ) -> Result<(), ComputeRectsError> {
        let source_can_enter_column = match self.source {
            DropSource::Panel { .. } => true,
            DropSource::Group { .. } => {
                matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
                    || matches!(
                        self.layout.node(self.source_root),
                        Some(Node::Split(split)) if split.axis == crate::Axis::Y
                    )
            }
            DropSource::Composition { .. } => false,
        };
        if !source_can_enter_column
            || !source_allows(self.layout, self.source_panels, AllowedArea::FLOATING)
        {
            return Ok(());
        }

        for column_rect in self.rects.columns.iter().filter(|column| {
            contains_node(self.tree_index, floating_root, column.column)
        }) {
            let Some(Node::Column(column)) = self.layout.node(column_rect.column) else {
                continue;
            };
            if column.mode.is_rail() {
                continue;
            }

            let bottom = column_rect
                .rect
                .checked_bottom()
                .map_err(ComputeRectsError::Geometry)?;
            let end_height = self
                .metrics
                .edge_drop_width
                .min(column_rect.rect.size.height / 3.0);
            let side_inset = self
                .metrics
                .edge_drop_width
                .min(column_rect.rect.size.width / 4.0);
            let end_rect = Rect::new(
                Point::new(column_rect.rect.origin.x + side_inset, bottom - end_height),
                Size::new(
                    (column_rect.rect.size.width - 2.0 * side_inset).max(0.0),
                    end_height,
                ),
            );
            let index = column.groups.len();
            if let DropSource::Group { group } = self.source
                && matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
                && self.layout.group_slot_is_noop(group, column_rect.column, index)
            {
                continue;
            }

            push_zone(
                self.zones,
                DropTarget::ColumnSlot {
                    column: column_rect.column,
                    index,
                },
                DropZone::ColumnEnd,
                end_rect,
            )?;
        }
        Ok(())
    }

    pub(super) fn append_side_zones(
        &mut self,
        target_groups: &[GroupRect],
        floating_root: NodeId,
    ) -> Result<(), ComputeRectsError> {
        // Individual Panel/Group horizontal splits remain Generic-only.
        // A whole Floating Column may join a compatible Column composition
        // in either profile, but only at the composition's outer edges.
        let target_columns = match self.layout.node(floating_root) {
            Some(Node::Column(_)) => Some((None, None)),
            Some(Node::Split(split))
                if split.axis == crate::Axis::X
                    && !split.children.is_empty()
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.layout.node(*child), Some(Node::Column(_)))) =>
            {
                Some((
                    split.children.first().copied(),
                    split.children.last().copied(),
                ))
            }
            _ => None,
        };
        let target_is_column_composition = target_columns.is_some();
        let source_can_split_horizontally = match self.source {
            DropSource::Panel { .. } => {
                self.layout.profile() == Profile::Generic || target_is_column_composition
            }
            DropSource::Group { .. } => {
                (self.layout.profile() == Profile::Generic
                    && (matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
                        || (source_is_floating_root(self.layout, self.source_root)
                            && matches!(
                                self.layout.node(self.source_root),
                                Some(Node::Split(crate::Split {
                                    axis: crate::Axis::X,
                                    ..
                                }))
                            ))))
                    || (target_is_column_composition
                        && matches!(self.layout.node(self.source_root), Some(Node::Tabs(_))))
            }
            DropSource::Composition { .. } => target_is_column_composition,
        };
        if !source_can_split_horizontally {
            return Ok(());
        }

        for target_group in target_groups {
            if matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group)
            {
                continue;
            }

            let (allow_left, allow_right) = if matches!(self.source, DropSource::Composition { .. }) {
                let Some(target_column) =
                    owner_column(self.layout, self.tree_index, target_group.group)
                else {
                    continue;
                };
                if matches!(
                    self.layout.node(target_column),
                    Some(Node::Column(column)) if column.mode.is_rail()
                ) {
                    continue;
                }
                match target_columns {
                    // A single-Column Floating composition exposes both sides
                    // along the full stack of Groups.
                    Some((None, None)) => (true, true),
                    // A horizontal composition exposes only its outermost
                    // left and right columns; interior columns are not targets.
                    Some((Some(left), Some(right))) => {
                        (target_column == left, target_column == right)
                    }
                    _ => continue,
                }
            } else {
                (true, true)
            };

            for (edge, allowed) in [
                (DropZone::Left, allow_left),
                (DropZone::Right, allow_right),
            ] {
                if !allowed
                    || group_edge_is_noop(
                        self.layout,
                        self.tree_index,
                        self.source,
                        target_group.group,
                        edge,
                    )
                {
                    continue;
                }
                push_zone(
                    self.zones,
                    DropTarget::GroupEdge {
                        group: target_group.group,
                        zone: edge,
                    },
                    edge,
                    group_edge_rect(target_group.rect, edge, self.metrics.edge_drop_width)?,
                )?;
            }
        }
        Ok(())
    }
}

fn floating_group_edge_rect(
    rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let content_top = (metrics.header_height + metrics.tab_strip_height).min(rect.size.height);
    let content_height = (rect.size.height - content_top).max(0.0);
    let height = metrics.edge_drop_width.min(content_height / 2.0);
    let bottom = rect.checked_bottom().map_err(ComputeRectsError::Geometry)?;
    let y = match edge {
        // The top edge starts below the Floating title and the group's tab strip.
        DropZone::Top => rect.origin.y + content_top,
        DropZone::Bottom => bottom - height,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let result = Rect::new(
        Point::new(rect.origin.x, y),
        Size::new(rect.size.width, height),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

impl FloatingZoneContext<'_> {
    fn append_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        root: NodeId,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        let source_can_split_vertically = match self.source {
            DropSource::Panel { .. } => true,
            DropSource::Group { .. } => {
                matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
                    || source_is_floating_root(self.layout, self.source_root)
            }
            DropSource::Composition { .. } => true,
        };
        if !source_can_split_vertically {
            return Ok(());
        }

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
