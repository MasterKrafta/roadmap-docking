use super::*;

impl Layout {
    pub(in crate::compute_drop_zones) fn append_main_group_zones(
        &self,
        source: DropSource,
        source_root: NodeId,
        source_panels: &[PanelId],
        tree_index: &DropZoneTreeIndex,
        rects: &LayoutRects,
        metrics: &DockMetrics,
        left_columns: &[(NodeId, Rect)],
        right_columns: &[(NodeId, Rect)],
        may_add_column: bool,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        // Main-dock groups: center/tab insertion, vertical group splitting,
        // and profile-correct horizontal/column splitting.
        for group_rect in &rects.groups {
            if !matches!(self.node(group_rect.group), Some(Node::Tabs(_))) {
                continue;
            }
            if is_rail_group(self, tree_index, group_rect.group) {
                continue;
            }
            let Some(area) = target_area(self, tree_index, group_rect.group) else {
                continue;
            };
            if area == AllowedArea::FLOATING {
                continue;
            }

            let tabs = match self.node(group_rect.group) {
                Some(Node::Tabs(tabs)) => tabs,
                _ => continue,
            };
            let source_contains_target = matches!(source, DropSource::Group { .. })
                && contains_node(tree_index, source_root, group_rect.group);
            let source_panel_is_here = match source {
                DropSource::Panel { panel } => {
                    panel_node(tree_index, panel).and_then(|node| direct_parent(tree_index, node))
                        == Some(group_rect.group)
                }
                DropSource::Group { .. } => false,
            };
            if !source_contains_target
                && !source_panel_is_here
                && source_allows(self, &source_panels, area)
            {
                let center_rect = group_center_rect(group_rect.rect, metrics)?;
                let target = match source {
                    DropSource::Panel { .. } => DropTarget::Tabs {
                        group: group_rect.group,
                        index: tabs.children.len(),
                    },
                    DropSource::Group { .. } => DropTarget::GroupTabs {
                        group: group_rect.group,
                        index: tabs.children.len(),
                    },
                };
                push_zone(zones, target, DropZone::Center, center_rect)?;
            }

            if source_contains_target
                || (matches!(source, DropSource::Group { .. })
                    && contains_node(tree_index, group_rect.group, source_root))
                || !source_allows(self, &source_panels, area)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Top, DropZone::Top),
                (DropZone::Bottom, DropZone::Bottom),
            ] {
                if group_edge_is_noop(self, tree_index, source, group_rect.group, edge) {
                    continue;
                }
                let target = DropTarget::GroupEdge {
                    group: group_rect.group,
                    zone: edge,
                };
                push_zone(
                    zones,
                    target,
                    kind,
                    group_edge_rect(group_rect.rect, edge, metrics.edge_drop_width)?,
                )?;
            }

            let left_edge =
                group_edge_rect(group_rect.rect, DropZone::Left, metrics.edge_drop_width)?;
            let right_edge =
                group_edge_rect(group_rect.rect, DropZone::Right, metrics.edge_drop_width)?;
            if let Some((side, column_index, _column_rect)) =
                group_main_column(self, tree_index, group_rect.group, &left_columns, &right_columns)
            {
                if may_add_column {
                    let left_target = match source {
                        DropSource::Panel { .. } => DropTarget::MainColumn {
                            side,
                            index: column_index,
                        },
                        DropSource::Group { .. } => DropTarget::GroupEdge {
                            group: group_rect.group,
                            zone: DropZone::Left,
                        },
                    };
                    let right_target = match source {
                        DropSource::Panel { .. } => DropTarget::MainColumn {
                            side,
                            index: column_index + 1,
                        },
                        DropSource::Group { .. } => DropTarget::GroupEdge {
                            group: group_rect.group,
                            zone: DropZone::Right,
                        },
                    };
                    if !group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        group_rect.group,
                        DropZone::Left,
                    ) {
                        push_zone(zones, left_target, DropZone::ColumnLeft, left_edge)?;
                    }
                    if !group_edge_is_noop(
                        self,
                        tree_index,
                        source,
                        group_rect.group,
                        DropZone::Right,
                    ) {
                        push_zone(zones, right_target, DropZone::ColumnRight, right_edge)?;
                    }
                }
            } else if self.profile() == Profile::Generic {
                // Generic layouts permit a nested horizontal Split around a group.
                for (edge, rect) in [(DropZone::Left, left_edge), (DropZone::Right, right_edge)] {
                    if !group_edge_is_noop(self, tree_index, source, group_rect.group, edge) {
                        push_zone(
                            zones,
                            DropTarget::GroupEdge {
                                group: group_rect.group,
                                zone: edge,
                            },
                            edge,
                            rect,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}
