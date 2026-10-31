//! Floating command dispatch, creation, docking, geometry, and stacking order.

use super::helpers::{geometry_error_reason, normalized_ratios, side_index};
use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn apply_floating_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::Float { node, bounds } => self.apply_float_command(node, bounds),
            DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. } => {
                self.apply_floating_window_command(command, metrics)
            }
            DockCommand::DockFloating { group, target } => {
                self.apply_dock_floating_command(group, target, metrics)
            }
            DockCommand::CloseFloatingGroup { group } => {
                self.apply_close_floating_group_command(group)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "floating command dispatcher received a non-floating command",
            }),
        }
    }

    pub(super) fn apply_close_floating_group_command(
        &mut self,
        group: &FloatingGroupId,
    ) -> Result<CommandApplication, CommandError> {
        let floating = self
            .floating_groups
            .get(*group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
        let area =
            self.dock_areas
                .get(floating.area)
                .copied()
                .ok_or(CommandError::InvalidTarget {
                    reason: "floating group references a missing dock area",
                })?;

        self.remember_subtree_dock_locations(area.root);
        self.floating_groups.remove(*group);
        self.dock_areas.remove(floating.area);
        self.remove_subtree(area.root);
        Ok(CommandApplication::persistent())
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
        let supports_photoshop_floating_root =
            matches!(self.nodes.get(*node), Some(Node::Panel(_) | Node::Tabs(_)));
        if self.profile == Profile::Photoshop && !supports_photoshop_floating_root {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "only a Panel or Tabs node can become a Photoshop floating root",
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
        self.validate_floating_dock_target(root, *target, metrics)?;
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
        self.validate_floating_dock_target(root, target, metrics)?;
        if target
            .referenced_node_id()
            .is_some_and(|node| self.is_descendant_or_same(root, node))
        {
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
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index })
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                self.dock_floating_composition_to_main_column(group_id, root, split, side, index)
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
            (
                Some(Node::Column(_)) | Some(Node::Split(_)),
                DropTarget::GroupEdge {
                    group: target_group,
                    zone: zone @ (DropZone::Left | DropZone::Right),
                },
            ) if self.profile == Profile::Generic
                && self.is_column_composition_root(root)
                && self.floating_group_is_in_column_composition(target_group) =>
            {
                self.dock_floating_composition_to_edge(group_id, root, target_group, zone)
            }
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

    /// Moves every Column in a horizontal floating composition into a main dock.
    ///
    /// The existing Column and Tabs nodes remain intact. Only the floating
    /// composition root is replaced with a main-dock Split(X) root.
    fn dock_floating_composition_to_main_column(
        &mut self,
        floating_group_id: FloatingGroupId,
        root: NodeId,
        split: Split,
        side: DockSide,
        index: usize,
    ) -> Result<(), CommandError> {
        if split.axis != Axis::X
            || split.children.is_empty()
            || split
                .children
                .iter()
                .any(|child| !matches!(self.nodes.get(*child), Some(Node::Column(_))))
        {
            return Err(CommandError::InvalidTarget {
                reason: "floating main-dock composition must contain horizontal Columns",
            });
        }

        let floating = self.floating_groups.get(floating_group_id).copied().ok_or(
            CommandError::MissingFloatingGroup {
                group: floating_group_id,
            },
        )?;
        let source_columns = split.children;
        let source_ratios = normalized_ratios(split.ratios.into_iter().map(Some).collect());
        let side_slot = side_index(side);
        let Some(main_area) = self.main_area(side).copied() else {
            if index != 0 {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "main-column insertion index is outside the empty dock",
                });
            }
            self.detach_node(root)?;
            let area_id = self.dock_areas.insert(crate::DockArea::main(side, root));
            self.main_areas[side_slot] = Some(area_id);
            return Ok(());
        };

        let main_root = main_area.root;
        let main_columns = self.main_columns(side)?;
        if index > main_columns.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "main-column insertion index is outside the dock",
            });
        }
        let (main_ratios, main_root_is_split) = match self.nodes.get(main_root) {
            Some(Node::Column(_)) => (vec![1.0], false),
            Some(Node::Split(main_split)) if main_split.axis == Axis::X => (
                normalized_ratios(main_split.ratios.iter().copied().map(Some).collect()),
                true,
            ),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "main dock root must be a Column or Split(X) of Columns",
                });
            }
            None => {
                return Err(CommandError::InvalidTarget {
                    reason: "main dock root is missing",
                });
            }
        };

        let main_width = main_columns
            .iter()
            .map(|column_id| match self.nodes.get(*column_id) {
                Some(Node::Column(column)) => Ok(column.width),
                _ => Err(CommandError::InvalidTarget {
                    reason: "main horizontal split must contain only Column nodes",
                }),
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum::<f32>();
        let source_width = floating.bounds.size.width;
        let combined_width = main_width + source_width;
        if !combined_width.is_finite() || main_width <= 0.0 || source_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "combined main and floating dock widths must be finite and positive",
            });
        }

        let main_weight = main_width / combined_width;
        let source_weight = source_width / combined_width;
        let mut children = main_columns;
        let mut ratios = main_ratios
            .into_iter()
            .map(|ratio| ratio * main_weight)
            .collect::<Vec<_>>();
        children.splice(index..index, source_columns);
        ratios.splice(
            index..index,
            source_ratios.into_iter().map(|ratio| ratio * source_weight),
        );
        let ratios = normalized_ratios(ratios.into_iter().map(Some).collect());

        self.detach_node(root)?;
        self.nodes.remove(root);
        let new_root = self.nodes.insert(Node::Split(crate::Split {
            axis: Axis::X,
            children,
            ratios,
        }));
        self.set_area_root(main_root, new_root)?;
        if main_root_is_split {
            self.nodes.remove(main_root);
        }

        Ok(())
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
                target_tabs.active_index = insert_at + source_active_index.min(moving_count - 1);
            }
        } else {
            return Err(CommandError::InvalidTarget {
                reason: "tab target changed unexpectedly",
            });
        }
        Ok(())
    }

    /// Validates a floating-group drop. Horizontal GroupEdge targets inside
    /// floating areas are reserved for joining complete Photoshop compositions;
    /// the general GroupEdge validator remains scoped to main-dock splits.
    fn is_column_composition_root(&self, root: NodeId) -> bool {
        match self.nodes.get(root) {
            Some(Node::Column(_)) => true,
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .iter()
                .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))),
            _ => false,
        }
    }

    fn floating_group_is_in_column_composition(&self, node: NodeId) -> bool {
        self.find_floating_root_containing(node)
            .is_some_and(|root| self.is_column_composition_root(root))
    }

    fn validate_floating_dock_target(
        &self,
        source_root: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let source_is_column_composition = self.is_column_composition_root(source_root);
        if let DropTarget::GroupEdge {
            group,
            zone: zone @ (DropZone::Left | DropZone::Right),
        } = target
            && self.profile == Profile::Generic
            && source_is_column_composition
            && self.floating_group_is_in_column_composition(group)
        {
            return self.validate_floating_composition_edge_target(group, zone);
        }

        self.validate_drop_target(target, metrics)
    }

    fn validate_floating_composition_edge_target(
        &self,
        group: NodeId,
        zone: DropZone,
    ) -> Result<(), CommandError> {
        if !matches!(zone, DropZone::Left | DropZone::Right) {
            return Err(CommandError::InvalidTarget {
                reason: "floating composition docking requires a left or right edge",
            });
        }
        if self.is_group_in_icon_rail(group) {
            return Err(CommandError::InvalidTarget {
                reason: "floating composition edge target cannot be an icon rail slot",
            });
        }
        if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
            return Err(match self.nodes.get(group) {
                Some(_) => CommandError::InvalidTarget {
                    reason: "floating composition edge target must be a Tabs node",
                },
                None => CommandError::MissingNode { node: group },
            });
        }
        let column = self
            .find_owner_column(group)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating composition edge target must belong to a Column",
            })?;
        let root =
            self.find_floating_root_containing(group)
                .ok_or(CommandError::InvalidTarget {
                    reason: "floating composition edge target must belong to another floating area",
                })?;
        let is_outer_edge = match self.nodes.get(root) {
            Some(Node::Column(_)) => root == column,
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                let index = split
                    .children
                    .iter()
                    .position(|child| *child == column)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "floating target column is not part of its composition",
                    })?;
                if zone == DropZone::Left {
                    index == 0
                } else {
                    index + 1 == split.children.len()
                }
            }
            _ => false,
        };
        if !is_outer_edge {
            return Err(CommandError::InvalidTarget {
                reason: "floating horizontal docking is allowed only at the outer left or right edge of a Column composition",
            });
        }
        Ok(())
    }

    /// Joins two Photoshop floating areas side-by-side while retaining each
    /// area's Columns as independent children of one horizontal composition.
    ///
    /// The destination FloatingGroup and its DockArea remain the window host;
    /// the source FloatingGroup is removed when its root is detached.
    fn dock_floating_composition_to_edge(
        &mut self,
        source_group_id: FloatingGroupId,
        source_root: NodeId,
        target_group: NodeId,
        zone: DropZone,
    ) -> Result<(), CommandError> {
        let source_floating = self.floating_groups.get(source_group_id).copied().ok_or(
            CommandError::MissingFloatingGroup {
                group: source_group_id,
            },
        )?;
        let target_root = self.find_floating_root_containing(target_group).ok_or(
            CommandError::InvalidTarget {
                reason: "side-by-side composition target must belong to another floating area",
            },
        )?;
        if target_root == source_root {
            return Ok(());
        }
        let target_group_id = self
            .floating_groups
            .iter()
            .find_map(|(id, floating)| {
                self.dock_areas
                    .get(floating.area)
                    .is_some_and(|area| area.root == target_root)
                    .then_some(id)
            })
            .ok_or(CommandError::InvalidTarget {
                reason: "side-by-side composition target has no floating window",
            })?;
        let target_floating = self.floating_groups.get(target_group_id).copied().ok_or(
            CommandError::MissingFloatingGroup {
                group: target_group_id,
            },
        )?;

        let (source_columns, source_ratios, source_was_split) = match self.nodes.get(source_root) {
            Some(Node::Column(_)) => (vec![source_root], vec![1.0], false),
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                (split.children.clone(), split.ratios.clone(), true)
            }
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "source floating area must be a Column or horizontal composition of Columns",
                });
            }
        };
        let (target_columns, target_ratios, target_was_split) = match self.nodes.get(target_root) {
            Some(Node::Column(_)) => (vec![target_root], vec![1.0], false),
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                (split.children.clone(), split.ratios.clone(), true)
            }
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "target floating area must be a Column or horizontal composition of Columns",
                });
            }
        };

        let target_column =
            self.find_owner_column(target_group)
                .ok_or(CommandError::InvalidTarget {
                    reason: "target group is not inside a floating Column",
                })?;
        let target_index = target_columns
            .iter()
            .position(|column| *column == target_column)
            .ok_or(CommandError::InvalidTarget {
                reason: "target group is not inside the target composition",
            })?;
        let is_outer_edge = match zone {
            DropZone::Left => target_index == 0,
            DropZone::Right => target_index + 1 == target_columns.len(),
            _ => false,
        };
        if !is_outer_edge {
            return Err(CommandError::InvalidTarget {
                reason: "floating composition may only be joined at its outer left or right edge",
            });
        }

        let source_ratios = normalized_ratios(source_ratios.into_iter().map(Some).collect());
        let target_ratios = normalized_ratios(target_ratios.into_iter().map(Some).collect());
        let combined_width = source_floating.bounds.size.width + target_floating.bounds.size.width;
        if !combined_width.is_finite() || combined_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "combined floating composition width must be finite and positive",
            });
        }
        let source_weight = source_floating.bounds.size.width / combined_width;
        let target_weight = target_floating.bounds.size.width / combined_width;

        let mut children = Vec::with_capacity(source_columns.len() + target_columns.len());
        let mut ratios = Vec::with_capacity(source_ratios.len() + target_ratios.len());
        if zone == DropZone::Left {
            ratios.extend(source_ratios.into_iter().map(|ratio| ratio * source_weight));
            children.extend(source_columns);
            ratios.extend(target_ratios.into_iter().map(|ratio| ratio * target_weight));
            children.extend(target_columns);
        } else {
            ratios.extend(target_ratios.into_iter().map(|ratio| ratio * target_weight));
            children.extend(target_columns);
            ratios.extend(source_ratios.into_iter().map(|ratio| ratio * source_weight));
            children.extend(source_columns);
        }

        // Detaching the source root unregisters its FloatingGroup and DockArea,
        // but deliberately leaves its Column/Panel nodes alive for the new tree.
        self.detach_node(source_root)?;
        if source_was_split {
            self.nodes.remove(source_root);
        }

        let composition_root = self.nodes.insert(Node::Split(crate::Split {
            axis: Axis::X,
            children,
            ratios,
        }));
        self.set_area_root(target_root, composition_root)?;
        if target_was_split {
            self.nodes.remove(target_root);
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
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::FloatingBounds(actual_bounds)),
                inverse: None,
                persistent: true,
            });
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds = actual_bounds;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::FloatingBounds(actual_bounds)),
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
