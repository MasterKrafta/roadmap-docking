//! Floating command dispatch, creation, docking, geometry, and stacking order.

use super::*;
use super::helpers::{geometry_error_reason, normalized_ratios};
use crate::AllowedArea;

impl Layout {
    pub(super) fn apply_floating_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::Float { node, bounds } => self.apply_float_command(node, bounds),
            DockCommand::FloatGroupAsColumn { group, bounds } => {
                self.apply_float_group_as_column_command(*group, *bounds, metrics)
            }
            DockCommand::FloatPanelAsGroup { panel, bounds } => {
                self.apply_float_panel_as_group_command(*panel, *bounds, metrics)
            }
            DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. } => {
                self.apply_floating_window_command(command, metrics)
            }
            DockCommand::DockFloating { group, target } => {
                self.apply_dock_floating_command(group, target, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "floating command dispatcher received a non-floating command",
            }),
        }
    }

    pub(super) fn apply_floating_window_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveFloating { group, origin } => {
                self.apply_move_floating_command(*group, *origin)
            }
            DockCommand::ResizeFloating { group, size } => {
                self.apply_resize_floating_command(*group, *size, metrics)
            }
            DockCommand::ResizeFloatingBounds { group, bounds } => {
                self.apply_resize_floating_bounds_command(*group, *bounds, metrics)
            }
            DockCommand::RaiseFloating { group } => self.apply_raise_floating_command(*group),
            _ => Err(CommandError::InvalidTarget {
                reason: "floating window dispatcher received a non-floating-window command",
            }),
        }
    }

    pub(super) fn apply_float_command(
        &mut self,
        node: &NodeId,
        bounds: &Rect,
    ) -> Result<CommandApplication, CommandError> {
        // Bounds use model-space logical pixels. Visible-screen clamping
        // belongs to the geometry/render adapter, not to Core/GPUI.
        if !self.nodes.contains_key(*node) {
            return Err(CommandError::MissingNode { node: *node });
        }
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        let moving_panels = self.panel_ids_in_subtree(*node);
        self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::FLOATING)?;
        let supports_photoshop_floating_root = match self.nodes.get(*node) {
            Some(Node::Panel(_) | Node::Tabs(_)) => true,
            Some(Node::Column(column)) => column
                .groups
                .iter()
                .all(|child| matches!(self.nodes.get(*child), Some(Node::Tabs(_)))),
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .iter()
                .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))),
            _ => false,
        };
        if self.profile == Profile::Photoshop && !supports_photoshop_floating_root {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "only a panel, tab group, column, or horizontal composition of columns can become a floating root",
            });
        }
        let next_z = self.next_floating_z_order()?;
        let panel_root = matches!(self.nodes.get(*node), Some(Node::Panel(_)));
        self.remember_subtree_dock_locations(*node);
        self.detach_node(*node)?;
        let root = if panel_root && self.profile == Profile::Photoshop {
            self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![*node],
                active_index: 0,
                collapsed: false,
            }))
        } else {
            *node
        };
        let area_id = self.dock_areas.insert(DockArea::floating(root));
        self.floating_groups
            .insert(FloatingGroup::new(area_id, *bounds, next_z));
        Ok(CommandApplication::persistent())
    }

    /// Detach a group from a multi-group Column and make it the sole group in
    /// a new floating Column. The caller decides whether detaching the group is
    /// structurally valid; this command still rejects a last-group detach.
    pub(super) fn apply_float_group_as_column_command(
        &mut self,
        group: NodeId,
        bounds: Rect,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if self.profile != Profile::Photoshop {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "floating a tab group as a Column is supported only by the Photoshop profile",
            });
        }
        if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
            return Err(CommandError::InvalidTarget {
                reason: "FloatGroupAsColumn requires a Tabs node",
            });
        }
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;

        let source_column_id = self.find_owner_column(group).ok_or(
            CommandError::InvalidTarget {
                reason: "FloatGroupAsColumn requires a group owned by a Column",
            },
        )?;
        let source_column = match self.nodes.get(source_column_id) {
            Some(Node::Column(column)) if column.groups.len() > 1 => column.clone(),
            Some(Node::Column(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "cannot detach the only group from its Column; float the containing composition instead",
                });
            }
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "group owner is not a Column",
                });
            }
        };
        let requested_width = if source_column.mode == ColumnMode::Expanded {
            source_column.width
        } else {
            source_column
                .last_expanded_width
                .unwrap_or(metrics.default_column_width)
        };
        let width = if requested_width.is_finite() && requested_width > 0.0 {
            requested_width.max(metrics.min_column_width)
        } else {
            metrics.default_column_width
        };

        let moving_panels = self.panel_ids_in_subtree(group);
        self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::FLOATING)?;
        let next_z = self.next_floating_z_order()?;
        self.remember_subtree_dock_locations(group);
        self.detach_node(group)?;

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

    /// Detach a panel from a group that will retain other tabs, then wrap the
    /// detached panel in a one-tab Tabs node and a standard single-group Column.
    pub(super) fn apply_float_panel_as_group_command(
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
}

impl Layout {
    pub(super) fn apply_dock_floating_command(
        &mut self,
        group: &FloatingGroupId,
        target: &DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        let root = self
            .floating_groups
            .get(*group)
            .and_then(|floating| self.dock_areas.get(floating.area).map(|area| area.root))
            .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
        self.validate_drop_target(*target, metrics)?;
        let panel_nodes = self.panel_nodes_in_subtree(root);
        let moving_panels = self.panel_ids_in_subtree(root);
        self.validate_panel_ids_allowed_for_target(&moving_panels, *target)?;
        self.dock_floating_group(*group, *target, metrics)?;
        for panel_node in panel_nodes {
            self.remember_panel_dock_location(panel_node);
        }
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
        if let DropTarget::Tabs { .. } | DropTarget::GroupTabs { .. } = target
            && matches!(self.nodes.get(root), Some(Node::Split(split)) if split.axis == Axis::Y)
        {
            return Err(CommandError::InvalidTarget {
                reason: "a floating stack cannot be merged into a tab strip; target a dock column or rail slot",
            });
        }
        match (self.nodes.get(root).cloned(), target) {
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index })
                if split.axis == Axis::Y =>
            {
                self.dock_floating_stack_to_main_column(root, split, side, index, metrics)
            }
            (Some(Node::Split(split)), DropTarget::RailSlot { column, index })
                if split.axis == Axis::Y =>
            {
                self.dock_floating_stack_to_rail_slot(root, split, column, index)
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
                self.dock_floating_stack_to_column_slot(root, split, column, index)
            }
            (
                Some(Node::Tabs(tabs)),
                DropTarget::Tabs {
                    group: target_group,
                    index,
                },
            ) => self.dock_floating_tabs_to_tabs(root, tabs, target_group, index),
            (Some(Node::Split(_)), _) => Err(CommandError::InvalidTarget {
                reason: "floating split stacks can only be docked into a main column or icon rail",
            }),
            (Some(_), _) => self.dock_floating_root_to_target(root, target, metrics),
            (None, _) => Err(CommandError::MissingNode { node: root }),
        }
    }

    pub(super) fn dock_floating_stack_to_main_column(
        &mut self,
        root: NodeId,
        split: Split,
        side: DockSide,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
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

    pub(super) fn dock_floating_stack_to_rail_slot(
        &mut self,
        root: NodeId,
        split: Split,
        column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
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

    pub(super) fn dock_floating_stack_to_column_slot(
        &mut self,
        root: NodeId,
        split: Split,
        column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
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

    pub(super) fn dock_floating_tabs_to_tabs(
        &mut self,
        root: NodeId,
        mut tabs: Tabs,
        target_group: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
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

    pub(super) fn dock_floating_root_to_target(
        &mut self,
        root: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.detach_node(root)?;
        self.insert_node_for_target(root, target, metrics, Some(0.5))
    }
}

impl Layout {
    pub(super) fn apply_move_floating_command(
        &mut self,
        group: FloatingGroupId,
        origin: Point,
    ) -> Result<CommandApplication, CommandError> {
        // Do not clamp to a monitor/work-area here: the adapter applies
        // its visible-area policy; Core stores finite logical coordinates.
        if !origin.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "origin",
                reason: "origin coordinates must be finite",
            });
        }
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let bounds = Rect::new(origin, floating.bounds.size);
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "origin",
                reason: geometry_error_reason(error),
            })?;
        if floating.bounds.origin == origin {
            return Ok(CommandApplication::persistent());
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds.origin = origin;
        }
        Ok(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::MoveFloating {
                group,
                origin: floating.bounds.origin,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_resize_floating_command(
        &mut self,
        group: FloatingGroupId,
        size: Size,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        size.validate_positive()
            .map_err(|error| CommandError::InvalidValue {
                field: "size",
                reason: geometry_error_reason(error),
            })?;
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        let minimum = self.minimum_floating_size(root, metrics)?;
        let actual = Size::new(
            size.width.max(minimum.width),
            size.height.max(minimum.height),
        );
        let new_bounds = Rect::new(floating.bounds.origin, actual);
        new_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "size",
                reason: geometry_error_reason(error),
            })?;
        if actual == floating.bounds.size {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::FloatingSize(actual)),
                inverse: None,
                persistent: true,
            });
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds = new_bounds;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::FloatingSize(actual)),
            inverse: Some(DockCommand::ResizeFloating {
                group,
                size: floating.bounds.size,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_resize_floating_bounds_command(
        &mut self,
        group: FloatingGroupId,
        bounds: Rect,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        let minimum = self.minimum_floating_size(root, metrics)?;
        let actual_bounds = Rect::new(
            bounds.origin,
            Size::new(
                bounds.size.width.max(minimum.width),
                bounds.size.height.max(minimum.height),
            ),
        );
        actual_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        if actual_bounds == floating.bounds {
            return Ok(CommandApplication::persistent());
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds = actual_bounds;
        }
        Ok(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::ResizeFloatingBounds {
                group,
                bounds: floating.bounds,
            }),
            persistent: true,
        })
    }

    pub(super) fn next_floating_z_order(&self) -> Result<i32, CommandError> {
        match self
            .floating_groups
            .values()
            .map(|group| group.z_order)
            .max()
        {
            Some(value) => value.checked_add(1).ok_or(CommandError::InvalidValue {
                field: "z_order",
                reason: "floating z-order overflow",
            }),
            None => Ok(0),
        }
    }

    pub(super) fn apply_raise_floating_command(
        &mut self,
        group: FloatingGroupId,
    ) -> Result<CommandApplication, CommandError> {
        let current = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let max_other = self
            .floating_groups
            .iter()
            .filter(|(other_id, _)| *other_id != group)
            .map(|(_, value)| value.z_order)
            .max();
        if max_other.is_none_or(|max| current.z_order > max) {
            return Ok(CommandApplication::persistent());
        }
        let next_z = self.next_floating_z_order()?;
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.z_order = next_z;
        }
        Ok(CommandApplication::persistent())
    }

    #[cfg(feature = "serde")]
    pub(super) fn repair_duplicate_floating_z_orders(&mut self) -> Vec<String> {
        let mut seen = HashSet::new();
        let has_duplicates = self
            .floating_groups
            .iter()
            .any(|(_, group)| !seen.insert(group.z_order));
        if !has_duplicates {
            return Vec::new();
        }

        let mut ordered: Vec<_> = self
            .floating_groups
            .iter()
            .map(|(group_id, group)| (group_id, group.z_order))
            .collect();
        ordered.sort_by_key(|(_, z_order)| *z_order);

        // A valid workspace cannot practically contain more than i32::MAX
        // floating groups. Keep the old invalid values if it somehow does.
        if ordered.len() > i32::MAX as usize {
            return Vec::new();
        }

        let mut notes = Vec::new();
        for (index, (group_id, old_z_order)) in ordered.into_iter().enumerate() {
            let new_z_order = index as i32;
            if old_z_order == new_z_order {
                continue;
            }
            if let Some(group) = self.floating_groups.get_mut(group_id) {
                group.z_order = new_z_order;
                notes.push(format!(
                    "floating group {group_id:?}: z_order changed from {old_z_order} to {new_z_order} to remove duplicate stacking order"
                ));
            }
        }
        notes
    }

    /// Returns floating windows in paint order: lowest z_order first (back-to-front).
    ///
    /// Render adapters should draw windows in this order. Valid layouts require
    /// unique z_order values, so stacking order is unambiguous.
    pub fn floating_groups_in_paint_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by_key(|(_, group)| group.z_order);
        groups
    }

    /// Returns floating windows in hit-test order: highest z_order first.
    ///
    /// Input adapters should test the first returned window before the rest.
    pub fn floating_groups_in_hit_test_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by_key(|(_, group)| std::cmp::Reverse(group.z_order));
        groups
    }
}
