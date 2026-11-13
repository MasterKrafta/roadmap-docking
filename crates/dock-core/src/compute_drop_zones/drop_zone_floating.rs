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

fn floating_column_splitter_drop_rect(
    splitter: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let width = metrics.edge_drop_width.max(splitter.size.width);
    let center_x = splitter.origin.x + splitter.size.width * 0.5;
    let rect = Rect::new(
        Point::new(center_x - width * 0.5, splitter.origin.y),
        Size::new(width, splitter.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(rect)
}

fn floating_side_edge_rect(
    floating_bounds: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let width = metrics.edge_drop_width.min(floating_bounds.size.width / 3.0);
    let x = match edge {
        DropZone::Left => floating_bounds.origin.x,
        DropZone::Right => floating_bounds.origin.x + floating_bounds.size.width - width,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let rect = Rect::new(
        Point::new(x, floating_bounds.origin.y),
        Size::new(width, floating_bounds.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(rect)
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
        if !source_allows(self.layout, self.source_panels, AllowedArea::FLOATING)
            || (matches!(self.source, DropSource::Composition { .. })
                && area.root == self.source_root)
        {
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
        self.append_column_boundary_zones(&target_groups, area.root)?;
        self.append_edge_zones(floating_rect, area.root, &target_groups)?;
        // Side targets are source-aware: Photoshop uses composition-wide edges,
        // while Generic retains its flexible per-Group horizontal split zones.
        self.append_side_zones(floating_rect, &target_groups, area.root)?;
        Ok(())
    }
}

impl FloatingZoneContext<'_> {
    pub(super) fn append_center_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        for target_group in target_groups {
            super::append_group_center_zone(
                self.layout,
                self.source,
                self.source_root,
                self.source_panels,
                self.tree_index,
                *target_group,
                AllowedArea::FLOATING,
                self.metrics,
                DropZone::FloatingCenter,
                self.zones,
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
                // A whole Composition uses FloatingTop/FloatingBottom for its
                // outer window boundaries. Main keeps one shared target per
                // splitter; Floating targets also expose each lower Group's own
                // Top edge so a complete composition can be inserted there.
                if matches!(self.source, DropSource::Composition { .. })
                    && let Some(column_id) =
                        owner_column(self.layout, self.tree_index, target_group.group)
                    && let Some(Node::Column(column)) = self.layout.node(column_id)
                    && let Some(group_index) = column
                        .groups
                        .iter()
                        .position(|group| *group == target_group.group)
                    && ((edge == DropZone::Top && group_index == 0)
                        || (edge == DropZone::Bottom
                            && group_index + 1 == column.groups.len()))
                {
                    continue;
                }
                if group_edge_is_noop(
                    self.layout,
                    self.tree_index,
                    self.source,
                    target_group.group,
                    edge,
                ) {
                    continue;
                }

                let zone_rect = match super::shared_group_vertical_edge_rect(
                    self.layout,
                    self.rects,
                    self.tree_index,
                    target_group.group,
                    target_group.rect,
                    edge,
                    self.metrics,
                )? {
                    Some(rect) => rect,
                    None
                        if edge == DropZone::Top
                            && matches!(self.source, DropSource::Composition { .. })
                            && target_area(self.layout, self.tree_index, target_group.group)
                                == Some(AllowedArea::FLOATING) =>
                    {
                        group_edge_rect(
                            target_group.rect,
                            edge,
                            self.metrics.edge_drop_width,
                        )?
                    }
                    None => continue,
                };

                push_zone(
                    self.zones,
                    DropTarget::GroupEdge {
                        group: target_group.group,
                        zone: edge,
                    },
                    edge,
                    zone_rect,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn append_column_slot_zones(
        &mut self,
        floating_root: NodeId,
    ) -> Result<(), ComputeRectsError> {
        let source_can_enter_column = matches!(
            self.source,
            DropSource::Composition { .. }
        ) && matches!(self.layout.node(self.source_root), Some(Node::Column(_)));
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

    /// Exposes one insertion zone for every physical splitter between
    /// sibling Columns in a Floating horizontal composition.
    ///
    /// The zone is intentionally distinct from the outer Left/Right side zones:
    /// it inserts before the Column on the right side of the selected splitter.
    pub(super) fn append_column_boundary_zones(
        &mut self,
        target_groups: &[GroupRect],
        floating_root: NodeId,
    ) -> Result<(), ComputeRectsError> {
        if self.layout.profile() != Profile::Photoshop
            || !source_allows(self.layout, self.source_panels, AllowedArea::FLOATING)
        {
            return Ok(());
        }

        let source_can_create_column = match self.source {
            DropSource::Panel { .. } => true,
            DropSource::Group { .. } => {
                matches!(self.layout.node(self.source_root), Some(Node::Tabs(_)))
            }
            DropSource::Composition { .. } => match self.layout.node(self.source_root) {
                Some(Node::Column(_)) => true,
                Some(Node::Split(split)) => {
                    split.axis == crate::Axis::X
                        && split.children.iter().all(|child| {
                            matches!(self.layout.node(*child), Some(Node::Column(_)))
                        })
                }
                _ => false,
            },
        };
        if !source_can_create_column {
            return Ok(());
        }

        let Some(Node::Split(split)) = self.layout.node(floating_root) else {
            return Ok(());
        };
        if split.axis != crate::Axis::X
            || split.children.len() < 2
            || !split.children.iter().all(|child| {
                matches!(self.layout.node(*child), Some(Node::Column(_)))
            })
        {
            return Ok(());
        }

        for splitter in self.rects.splitters.iter().filter(|splitter| {
            splitter.owner == floating_root && splitter.axis == crate::Axis::X
        }) {
            let Some(right_index) = splitter.boundary_index.checked_add(1) else {
                continue;
            };
            let (Some(left_column), Some(right_column)) = (
                split.children.get(splitter.boundary_index).copied(),
                split.children.get(right_index).copied(),
            ) else {
                continue;
            };
            let left_is_rail = matches!(
                self.layout.node(left_column),
                Some(Node::Column(column)) if column.mode.is_rail()
            );
            let right_is_rail = matches!(
                self.layout.node(right_column),
                Some(Node::Column(column)) if column.mode.is_rail()
            );
            if left_is_rail || right_is_rail {
                continue;
            }

            let Some(target_group) = target_groups
                .iter()
                .filter(|group| {
                    owner_column(self.layout, self.tree_index, group.group) == Some(right_column)
                })
                .min_by(|left, right| {
                    left.rect
                        .origin
                        .y
                        .partial_cmp(&right.rect.origin.y)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            else {
                continue;
            };

            push_zone(
                self.zones,
                DropTarget::GroupEdge {
                    group: target_group.group,
                    zone: DropZone::FloatingColumnInsertion,
                },
                DropZone::FloatingColumnInsertion,
                floating_column_splitter_drop_rect(splitter.rect, self.metrics)?,
            )?;
        }
        Ok(())
    }

    pub(super) fn append_side_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        target_groups: &[GroupRect],
        floating_root: NodeId,
    ) -> Result<(), ComputeRectsError> {
        // Photoshop uses the two outer Floating composition edges and spans
        // the full window height. Generic retains flexible per-Group side zones.
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

        if self.layout.profile() == Profile::Photoshop {
            // A source already inside this Floating composition must not create
            // an outer side target for its own window. Otherwise, side targets
            // belong to the composition's two outermost Columns, not to every
            // individual Group in the vertical stacks.
            if matches!(
                self.source,
                DropSource::Panel { .. } | DropSource::Group { .. }
            ) && contains_node(self.tree_index, floating_root, self.source_root)
            {
                return Ok(());
            }

            let (left_column, right_column) = match target_columns {
                Some((None, None)) => (floating_root, floating_root),
                Some((Some(left), Some(right))) => (left, right),
                _ => return Ok(()),
            };

            for (edge, target_column, first_in_stack) in [
                (DropZone::Left, left_column, true),
                (DropZone::Right, right_column, false),
            ] {
                let Some(Node::Column(column)) = self.layout.node(target_column) else {
                    continue;
                };
                if column.mode.is_rail() {
                    continue;
                }

                let target_group = target_groups
                    .iter()
                    .filter(|group| {
                        owner_column(self.layout, self.tree_index, group.group)
                            == Some(target_column)
                    })
                    .min_by(|left, right| {
                        let ordering = left
                            .rect
                            .origin
                            .y
                            .partial_cmp(&right.rect.origin.y)
                            .unwrap_or(std::cmp::Ordering::Equal);
                        if first_in_stack { ordering } else { ordering.reverse() }
                    });
                let Some(target_group) = target_group else {
                    continue;
                };

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
                    floating_side_edge_rect(floating_rect.rect, edge, self.metrics)?,
                )?;
            }
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

        let bottom_rect = floating_edge_rect(floating_rect.rect, DropZone::Bottom, self.metrics)?;
        let horizontal_composition = matches!(
            self.layout.node(root),
            Some(Node::Split(split)) if split.axis == crate::Axis::X
        );
        let whole_column_source = matches!(
            self.source,
            DropSource::Composition { .. }
        ) && matches!(self.layout.node(self.source_root), Some(Node::Column(_)));

        if horizontal_composition {
            self.append_horizontal_edge_zones(
                floating_rect,
                target_groups,
                !whole_column_source,
            )?;
        } else {
            let top_rect = if let Some(first_group) = target_groups.first() {
                floating_top_edge_rect(floating_rect.rect, first_group.rect, self.metrics)?
            } else {
                None
            };
            self.append_vertical_edge_zones(
                top_rect,
                bottom_rect,
                target_groups,
                !whole_column_source,
            )?;
        }
        Ok(())
    }

    fn append_horizontal_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        target_groups: &[GroupRect],
        include_bottom: bool,
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
            self.append_column_edge_zones(floating_rect, column, target_groups, include_bottom)?;
        }

        // Generic-profile horizontal splits may contain Tabs nodes directly,
        // without a containing Column. Clip their edge zones to each group.
        for target_group in target_groups
            .iter()
            .filter(|group| owner_column(self.layout, self.tree_index, group.group).is_none())
        {
            self.append_direct_branch_edge_zones(
                floating_rect.rect,
                *target_group,
                include_bottom,
            )?;
        }
        Ok(())
    }

    fn append_column_edge_zones(
        &mut self,
        floating_rect: FloatingGroupRect,
        column: NodeId,
        target_groups: &[GroupRect],
        include_bottom: bool,
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

        if let Some(column_top_rect) = floating_top_edge_rect_for_branch(
            floating_rect.rect,
            column_rect.rect,
            first_group.rect,
            self.metrics,
        )? {
            self.push_floating_edge_zone(
                first_group.group,
                DropZone::Top,
                DropZone::FloatingTop,
                column_top_rect,
            )?;
        }
        let column_bottom_rect = floating_edge_rect_for_branch(
            floating_rect.rect,
            column_rect.rect,
            DropZone::Bottom,
            self.metrics,
        )?;
        if include_bottom {
            self.push_floating_edge_zone(
                last_group.group,
                DropZone::Bottom,
                DropZone::FloatingBottom,
                column_bottom_rect,
            )?;
        }
        Ok(())
    }

    fn append_direct_branch_edge_zones(
        &mut self,
        floating_bounds: Rect,
        target_group: GroupRect,
        include_bottom: bool,
    ) -> Result<(), ComputeRectsError> {
        if let Some(top_branch_rect) = floating_top_edge_rect_for_branch(
            floating_bounds,
            target_group.rect,
            target_group.rect,
            self.metrics,
        )? {
            self.push_floating_edge_zone(
                target_group.group,
                DropZone::Top,
                DropZone::FloatingTop,
                top_branch_rect,
            )?;
        }
        let bottom_branch_rect = floating_edge_rect_for_branch(
            floating_bounds,
            target_group.rect,
            DropZone::Bottom,
            self.metrics,
        )?;
        if include_bottom {
            self.push_floating_edge_zone(
                target_group.group,
                DropZone::Bottom,
                DropZone::FloatingBottom,
                bottom_branch_rect,
            )?;
        }
        Ok(())
    }

    fn append_vertical_edge_zones(
        &mut self,
        top_rect: Option<Rect>,
        bottom_rect: Rect,
        target_groups: &[GroupRect],
        include_bottom: bool,
    ) -> Result<(), ComputeRectsError> {
        // A vertically stacked floating root has one outer top and bottom,
        // targeting its first and last groups respectively.
        let Some(first_group) = target_groups.first().copied() else {
            return Ok(());
        };
        let Some(last_group) = target_groups.last().copied() else {
            return Ok(());
        };
        if let Some(top_rect) = top_rect {
            self.push_floating_edge_zone(
                first_group.group,
                DropZone::Top,
                DropZone::FloatingTop,
                top_rect,
            )?;
        }
        if include_bottom {
            self.push_floating_edge_zone(
                last_group.group,
                DropZone::Bottom,
                DropZone::FloatingBottom,
                bottom_rect,
            )?;
        }
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
