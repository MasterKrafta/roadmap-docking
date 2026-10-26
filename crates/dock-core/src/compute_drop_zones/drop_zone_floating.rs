use super::*;

impl Layout {
    pub(super) fn append_floating_window_zones(
        &self,
        source: DropSource,
                source_root: NodeId,
                source_panels: &[PanelId],
                tree_index: &DropZoneTreeIndex,
                rects: &LayoutRects,
                metrics: &DockMetrics,
                zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        // Floating-window drop zones are stacked over their contents. Their outer
        // top/bottom targets address the first/last group in a vertical stack.
        for floating_rect in rects.floating_groups.iter().rev() {
            let Some(floating) = self.floating_group(floating_rect.group) else {
                continue;
            };
            let Some(area) = self.dock_area(floating.area) else {
                continue;
            };
            if !source_allows(self, &source_panels, AllowedArea::FLOATING) {
                continue;
            }
            let mut target_groups: Vec<_> = rects
                .groups
                .iter()
                .filter(|group| contains_node(tree_index, area.root, group.group))
                .collect();
            target_groups.sort_by(|left, right| {
                left.rect
                    .origin
                    .y
                    .partial_cmp(&right.rect.origin.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if target_groups.is_empty() {
                continue;
            }
            // A floating composition can contain multiple target groups at the same
            // vertical position (for example, two columns in a Split(X)). Build
            // center zones per group so point resolution picks the group actually
            // under the pointer instead of arbitrarily choosing the first branch.
            for target_group in &target_groups {
                let tabs = match self.node(target_group.group) {
                    Some(Node::Tabs(tabs)) if !tabs.collapsed => tabs,
                    _ => continue,
                };
                let is_self = matches!(source, DropSource::Group { .. })
                    && contains_node(tree_index, source_root, target_group.group);
                let panel_already_in_group = match source {
                    DropSource::Panel { panel } => {
                        panel_node(tree_index, panel)
                            .and_then(|node_id| direct_parent(tree_index, node_id))
                            == Some(target_group.group)
                    }
                    DropSource::Group { .. } => false,
                };
                if is_self || panel_already_in_group {
                    continue;
                }
                let target = match source {
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
                    zones,
                    target,
                    DropZone::FloatingCenter,
                    group_center_rect(target_group.rect, metrics)?,
                )?;
            }

            let top_rect = floating_edge_rect(floating_rect.rect, DropZone::Top, metrics)?;
            let bottom_rect = floating_edge_rect(floating_rect.rect, DropZone::Bottom, metrics)?;
            let horizontal_composition =
                matches!(self.node(area.root), Some(Node::Split(split)) if split.axis == crate::Axis::X);
            if horizontal_composition {
                // In a horizontal composition, the top and bottom edge zones
                // span each column independently. Thus dropping over the right
                // branch addresses its top/bottom group, not the left branch.
                let mut columns = Vec::new();
                for target_group in &target_groups {
                    let Some(column) = owner_column(self, tree_index, target_group.group) else {
                        continue;
                    };
                    if !columns.contains(&column) {
                        columns.push(column);
                    }
                }
                for column in columns {
                    let mut column_groups: Vec<_> = target_groups
                        .iter()
                        .filter(|group| {
                            owner_column(self, tree_index, group.group) == Some(column)
                        })
                        .collect();
                    column_groups.sort_by(|left, right| {
                        left.rect
                            .origin
                            .y
                            .partial_cmp(&right.rect.origin.y)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    let Some(first_group) = column_groups.first().copied() else {
                        continue;
                    };
                    let Some(last_group) = column_groups.last().copied() else {
                        continue;
                    };
                    let Some(column_rect) =
                        rects.columns.iter().find(|entry| entry.column == column)
                    else {
                        continue;
                    };
                    let column_top_rect = floating_edge_rect_for_branch(
                        floating_rect.rect,
                        column_rect.rect,
                        DropZone::Top,
                        metrics,
                    )?;
                    let column_bottom_rect = floating_edge_rect_for_branch(
                        floating_rect.rect,
                        column_rect.rect,
                        DropZone::Bottom,
                        metrics,
                    )?;

                    if !(matches!(source, DropSource::Group { .. })
                        && group_edge_is_noop(
                            self,
                            tree_index,
                            source,
                            first_group.group,
                            DropZone::Top,
                        ))
                    {
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: first_group.group,
                                zone: DropZone::Top,
                            },
                            DropZone::FloatingTop,
                            column_top_rect,
                        )?;
                    }
                    if !(matches!(source, DropSource::Group { .. })
                        && group_edge_is_noop(
                            self,
                            tree_index,
                            source,
                            last_group.group,
                            DropZone::Bottom,
                        ))
                    {
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: last_group.group,
                                zone: DropZone::Bottom,
                            },
                            DropZone::FloatingBottom,
                            column_bottom_rect,
                        )?;
                    }
                }

                // Generic-profile horizontal splits may contain Tabs nodes
                // directly, without a containing Column. Preserve edge targets for
                // those branches as well by clipping the zone to each group rect.
                for target_group in target_groups.iter().filter(|group| {
                    owner_column(self, tree_index, group.group).is_none()
                }) {
                    let top_branch_rect = floating_edge_rect_for_branch(
                        floating_rect.rect,
                        target_group.rect,
                        DropZone::Top,
                        metrics,
                    )?;
                    let bottom_branch_rect = floating_edge_rect_for_branch(
                        floating_rect.rect,
                        target_group.rect,
                        DropZone::Bottom,
                        metrics,
                    )?;
                    if !(matches!(source, DropSource::Group { .. })
                        && group_edge_is_noop(
                            self,
                            tree_index,
                            source,
                            target_group.group,
                            DropZone::Top,
                        ))
                    {
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: target_group.group,
                                zone: DropZone::Top,
                            },
                            DropZone::FloatingTop,
                            top_branch_rect,
                        )?;
                    }
                    if !(matches!(source, DropSource::Group { .. })
                        && group_edge_is_noop(
                            self,
                            tree_index,
                            source,
                            target_group.group,
                            DropZone::Bottom,
                        ))
                    {
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: target_group.group,
                                zone: DropZone::Bottom,
                            },
                            DropZone::FloatingBottom,
                            bottom_branch_rect,
                        )?;
                    }
                }
            } else {
                // A vertically stacked floating root has one outer top and
                // bottom: those zones target its first and last groups.
                target_groups.sort_by(|left, right| {
                    left.rect
                        .origin
                        .y
                        .partial_cmp(&right.rect.origin.y)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let first_group = target_groups.first().copied().unwrap();
                let last_group = target_groups.last().copied().unwrap();
                if !(matches!(source, DropSource::Group { .. })
                    && group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        first_group.group,
                        DropZone::Top,
                    ))
                {
                    push_zone(
                        zones,
                        DropTarget::GroupEdge {
                            group: first_group.group,
                            zone: DropZone::Top,
                        },
                        DropZone::FloatingTop,
                        top_rect,
                    )?;
                }
                if !(matches!(source, DropSource::Group { .. })
                    && group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        last_group.group,
                        DropZone::Bottom,
                    ))
                {
                    push_zone(
                        zones,
                        DropTarget::GroupEdge {
                            group: last_group.group,
                            zone: DropZone::Bottom,
                        },
                        DropZone::FloatingBottom,
                        bottom_rect,
                    )?;
                }
            }

            // Generic profile also supports horizontal nested splits within a floating area.
            if self.profile() == Profile::Generic {
                for target_group in target_groups {
                    if matches!(source, DropSource::Group { .. })
                        && contains_node(tree_index, source_root, target_group.group)
                    {
                        continue;
                    }
                    for (edge, kind) in [
                        (DropZone::Left, DropZone::Left),
                        (DropZone::Right, DropZone::Right),
                    ] {
                        if group_edge_is_noop(self, tree_index, source, target_group.group, edge) {
                            continue;
                        }
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: target_group.group,
                                zone: edge,
                            },
                            kind,
                            group_edge_rect(target_group.rect, edge, metrics.edge_drop_width)?,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}
