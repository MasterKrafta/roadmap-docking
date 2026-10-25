use super::*;
use super::helpers::{geometry_error_reason, normalized_ratios};
use crate::AllowedArea;

impl Layout {
    /// Detach a panel from a group that will retain other tabs, then wrap the
    /// detached panel in a one-tab Tabs node and a standard single-group Column.
    pub(super) fn float_panel_as_group(
        &mut self,
        panel: PanelId,
        bounds: Rect,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if self.profile != Profile::Photoshop {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "floating a tab as its own group is supported only by the Photoshop profile",
            });
        }
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        let panel_node = self
            .find_panel_node(panel)
            .ok_or(CommandError::MissingPanel { panel })?;
        let source_group = self.find_parent(panel_node).ok_or(
            CommandError::InvalidTarget {
                reason: "FloatPanelAsGroup requires a panel inside a Tabs group",
            },
        )?;
        match self.nodes.get(source_group) {
            Some(Node::Tabs(tabs))
                if tabs.children.len() > 1 && tabs.children.contains(&panel_node) => {}
            Some(Node::Tabs(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "cannot detach the only tab from a group; float the containing group instead",
                });
            }
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "FloatPanelAsGroup requires a panel inside a Tabs group",
                });
            }
        }
        let requested_width = self
            .find_owner_column(source_group)
            .and_then(|column_id| match self.nodes.get(column_id) {
                Some(Node::Column(column)) if column.mode == ColumnMode::Expanded => {
                    Some(column.width)
                }
                Some(Node::Column(column)) => column.last_expanded_width,
                _ => None,
            })
            .unwrap_or(metrics.default_column_width);
        let width = if requested_width.is_finite() && requested_width > 0.0 {
            requested_width.max(metrics.min_column_width)
        } else {
            metrics.default_column_width
        };

        let moving_panels = self.panel_ids_in_subtree(panel_node);
        self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::FLOATING)?;
        let next_z = self.next_floating_z_order()?;
        self.remember_subtree_dock_locations(panel_node);
        self.detach_node(panel_node)?;

        let group = self.nodes.insert(Node::Tabs(Tabs {
            children: vec![panel_node],
            active_index: 0,
            collapsed: false,
        }));
        let root = self.nodes.insert(Node::Column(Column {
            width,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
            last_expanded_width: Some(width),
        }));
        let area_id = self.dock_areas.insert(DockArea::floating(root));
        self.floating_groups
            .insert(FloatingGroup::new(area_id, bounds, next_z));
        Ok(CommandApplication::persistent())
    }

    pub(super) fn dock_floating_group(
        &mut self,
        group_id: FloatingGroupId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let floating = self
            .floating_groups
            .get(group_id)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: group_id })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        self.validate_drop_target(target, metrics)?;
        if target.referenced_node_id().is_some_and(|node| self.is_descendant_or_same(root, node)) {
            return Ok(());
        }
        if let DropTarget::Tabs { .. } | DropTarget::GroupTabs { .. } = target {
            if matches!(self.nodes.get(root), Some(Node::Split(split)) if split.axis == Axis::Y) {
                return Err(CommandError::InvalidTarget {
                    reason: "a floating stack cannot be merged into a tab strip; target a dock column or rail slot",
                });
            }
        }
        match (self.nodes.get(root).cloned(), target) {
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index })
                if split.axis == Axis::Y =>
            {
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "floating stack contains a non-Tabs group",
                    });
                }
                self.detach_node(root)?;
                self.nodes.remove(root);
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: split.children,
                    group_ratios: normalized_ratios(split.ratios.into_iter().map(Some).collect()),
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(side, index, column, None, metrics)
            }
            (Some(Node::Split(split)), DropTarget::RailSlot { column, index })
                if split.axis == Axis::Y =>
            {
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "floating stack contains a non-Tabs group",
                    });
                }
                let insert_at = self.rail_slot_group_index(column, index)?;
                self.detach_node(root)?;
                self.nodes.remove(root);
                for (insert_at, child) in (insert_at..).zip(split.children) {
                    self.insert_group_at_column(column, child, insert_at, None)?;
                }
                Ok(())
            }
            (
                Some(Node::Tabs(_)),
                DropTarget::GroupTabs {
                    group: target_group,
                    index,
                },
            ) => self.merge_group_into_tabs(root, target_group, index),
            (Some(Node::Split(split)), DropTarget::ColumnSlot { column, index })
                if split.axis == Axis::Y =>
            {
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "floating stack contains a non-Tabs group",
                    });
                }
                self.detach_node(root)?;
                self.nodes.remove(root);
                for (insert_at, child) in (index..).zip(split.children) {
                    self.insert_group_at_column(column, child, insert_at, None)?;
                }
                Ok(())
            }
            (
                Some(Node::Tabs(mut tabs)),
                DropTarget::Tabs {
                    group: target_group,
                    index,
                },
            ) => {
                self.detach_node(root)?;
                if let Some(Node::Tabs(old_root)) = self.nodes.get_mut(root) {
                    old_root.children.clear();
                    old_root.active_index = 0;
                }
                self.nodes.remove(root);
                if let Some(Node::Tabs(target_tabs)) = self.nodes.get_mut(target_group) {
                    let insert_at = index.min(target_tabs.children.len());
                    let source_active_index = tabs.active_index;
                    let moving_count = tabs.children.len();
                    for (offset, child) in tabs.children.drain(..).enumerate() {
                        target_tabs.children.insert(insert_at + offset, child);
                    }
                    if moving_count > 0 {
                        target_tabs.active_index =
                            insert_at + source_active_index.min(moving_count - 1);
                    }
                } else {
                    return Err(CommandError::InvalidTarget {
                        reason: "tab target changed unexpectedly",
                    });
                }
                Ok(())
            }
            (Some(Node::Split(_)), _) => Err(CommandError::InvalidTarget {
                reason: "floating split stacks can only be docked into a main column or icon rail",
            }),
            (Some(_), _) => {
                self.detach_node(root)?;
                self.insert_node_for_target(root, target, metrics, Some(0.5))
            }
            (None, _) => Err(CommandError::MissingNode { node: root }),
        }
    }
}
