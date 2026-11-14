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
            DockCommand::Float { node, bounds } => self.apply_float_command(node, bounds, metrics),
            DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::ResizeFloatingBoundsFromEdge { .. }
            | DockCommand::RaiseFloating { .. } => {
                self.apply_floating_window_command(command, metrics)
            }
            DockCommand::DockFloating { group, target } => {
                self.apply_dock_floating_command(group, target, metrics)
            }
            DockCommand::DockMainArea { side, target } => {
                self.apply_dock_main_area_command(side, target, metrics)
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
            DockCommand::ResizeFloatingBoundsFromEdge {
                group,
                bounds,
                edge,
                scale_factor,
            } => self.apply_resize_floating_bounds_from_edge_command(
                *group,
                *bounds,
                *edge,
                *scale_factor,
                metrics,
            ),
            DockCommand::RaiseFloating { group } => self.apply_raise_floating_command(*group),
            _ => Err(CommandError::InvalidTarget {
                reason: "floating window dispatcher received a non-floating-window command",
            }),
        }
    }

    pub(super) fn floating_root_for_node(
        &mut self,
        node: NodeId,
        requested_width: f32,
        metrics: &DockMetrics,
    ) -> Result<NodeId, CommandError> {
        if self.profile != Profile::Photoshop {
            return Ok(node);
        }

        let group = match self.nodes.get(node) {
            Some(Node::Column(_)) => return Ok(node),
            Some(Node::Tabs(_)) => node,
            Some(Node::Panel(_)) => self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![node],
                active_index: 0,
                collapsed: false,
            })),
            Some(Node::Split(_)) | None => return Ok(node),
        };

        let minimum_width = self.minimum_node_size(group, metrics)?.width;
        let width = requested_width.max(metrics.min_column_width).max(minimum_width);
        Ok(self.nodes.insert(Node::Column(crate::Column {
            width,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
            last_expanded_width: Some(width),
        })))
    }

    pub(super) fn apply_float_command(
        &mut self,
        node: &NodeId,
        bounds: &Rect,
        metrics: &DockMetrics,
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
            Some(Node::Panel(_) | Node::Tabs(_) | Node::Column(_)) => true,
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .iter()
                .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))),
            _ => false,
        };
        if self.profile == Profile::Photoshop && !supports_photoshop_floating_root {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "Photoshop floating compositions must be rooted at a Column or Split(X) of Columns",
            });
        }
        let next_z = self.next_floating_z_order()?;
        self.remember_subtree_dock_locations(*node);
        self.detach_node(*node)?;
        let root = self.floating_root_for_node(*node, bounds.size.width, metrics)?;
        let area_id = self.dock_areas.insert(DockArea::floating(root));
        let floating_group = self
            .floating_groups
            .insert(FloatingGroup::new(area_id, *bounds, next_z));
        Ok(CommandApplication {
            actual_value: Some(CommandValue::FloatingGroup(floating_group)),
            inverse: None,
            persistent: true,
        })
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

    pub(super) fn apply_dock_main_area_command(
        &mut self,
        side: &DockSide,
        target: &DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if self.profile != crate::Profile::Photoshop {
            return Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "docking a complete Main composition into Floating is available only in the Photoshop profile",
            });
        }

        let source_root = self
            .main_area(*side)
            .map(|area| area.root)
            .ok_or(CommandError::InvalidTarget {
                reason: "Main DockArea drag source no longer exists",
            })?;
        if !self.is_column_composition_root(source_root) {
            return Err(CommandError::InvalidTarget {
                reason: "Main DockArea source must be a Column or horizontal composition of Columns",
            });
        }

        let target_node = target.referenced_node_id().ok_or(CommandError::InvalidTarget {
            reason: "Main DockArea docking requires a Floating composition target",
        })?;
        let target_root = self
            .find_floating_root_containing(target_node)
            .ok_or(CommandError::InvalidTarget {
                reason: "Main DockArea can only dock into another Floating composition",
            })?;
        let target_bounds = self
            .floating_groups
            .iter()
            .find_map(|(_, floating)| {
                self.dock_areas
                    .get(floating.area)
                    .is_some_and(|area| area.root == target_root)
                    .then_some(floating.bounds)
            })
            .ok_or(CommandError::InvalidTarget {
                reason: "Floating target has no registered window bounds",
            })?;

        self.validate_floating_dock_target(source_root, *target, metrics)?;
        let moving_panels = self.panel_ids_in_subtree(source_root);
        self.validate_panel_ids_allowed_for_target(&moving_panels, *target)?;

        // Reuse the exact Floating -> Floating docking path. The temporary
        // source host exists only inside this transaction and is removed before
        // the candidate Layout can be committed.
        let temporary = self.apply_float_command(&source_root, &target_bounds, metrics)?;
        let temporary_group = match temporary.actual_value {
            Some(CommandValue::FloatingGroup(group)) => group,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "temporary Main DockArea float did not create a Floating group",
                });
            }
        };
        self.apply_dock_floating_command(&temporary_group, target, metrics)?;
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
            (Some(Node::Column(_)), DropTarget::MainColumn { side, index }) => {
                self.detach_node(root)?;
                self.insert_existing_column_at(side, index, root, None, metrics)
            }
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index })
                if split.axis == Axis::X =>
            {
                self.dock_floating_composition_to_main_column(group_id, root, split, side, index, metrics)
            }
            (Some(Node::Split(split)), DropTarget::RailSlot { column, index })
                if split.axis == Axis::Y =>
            {
                self.dock_floating_stack_to_rail_slot(root, split, column, index)
            }
            (
                Some(Node::Column(column)),
                DropTarget::GroupTabs {
                    group: target_group,
                    index,
                },
            ) if column.groups.len() == 1 => {
                self.merge_group_into_tabs(column.groups[0], target_group, index)
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
                Some(Node::Column(column)),
                DropTarget::Tabs {
                    group: target_group,
                    index,
                },
            ) if column.groups.len() == 1 => {
                match self.nodes.get(column.groups[0]).cloned() {
                    Some(Node::Tabs(tabs)) => self.dock_floating_tabs_to_tabs(
                        column.groups[0],
                        tabs,
                        target_group,
                        index,
                    ),
                    _ => Err(CommandError::InvalidTarget {
                        reason: "a floating Column dropped into Tabs must contain one Tabs group",
                    }),
                }
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
                    zone: zone @ (
                        DropZone::Left | DropZone::Right | DropZone::FloatingColumnInsertion
                    ),
                },
            ) if self.is_column_composition_root(root)
                && self.floating_group_is_in_column_composition(target_group) =>
            {
                self.dock_floating_composition_to_edge(
                    group_id,
                    root,
                    target_group,
                    zone,
                    metrics,
                )
            }
            (
                Some(Node::Column(column)),
                DropTarget::GroupEdge {
                    group: target_group,
                    zone: zone @ (DropZone::Left | DropZone::Right),
                },
            ) if self.profile == Profile::Photoshop
                && column.groups.len() == 1
                && self
                    .find_owner_column(target_group)
                    .is_some_and(|target_column| self.main_column_position(target_column).is_some()) =>
            {
                let moving_group = column.groups[0];
                self.detach_node(moving_group)?;
                self.insert_group_edge(moving_group, target_group, zone, Some(0.5), metrics)
            }
            (
                Some(Node::Column(_)) | Some(Node::Split(_)),
                DropTarget::GroupEdge {
                    group: target_group,
                    zone: zone @ (DropZone::Top | DropZone::Bottom),
                },
            ) if self.profile == Profile::Photoshop && self.is_column_composition_root(root) => {
                self.dock_floating_composition_to_group_edge(root, target_group, zone)
            }
            (
                Some(Node::Column(column)),
                DropTarget::ColumnSlot {
                    column: target_column,
                    index,
                },
            ) if self.profile == Profile::Photoshop && !column.groups.is_empty() => {
                self.dock_floating_column_to_column_slot(
                    root,
                    column,
                    target_column,
                    index,
                )
            }
            (
                Some(Node::Column(column)),
                DropTarget::RailSlot {
                    column: target_column,
                    index,
                },
            ) if self.profile == Profile::Photoshop && !column.groups.is_empty() => {
                self.dock_floating_column_to_rail_slot(
                    root,
                    column,
                    target_column,
                    index,
                )
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

    /// Moves every branch of a horizontal floating composition into a main dock.
    ///
    /// Existing Columns are preserved; other Generic-profile branches are wrapped
    /// in a Column without rebuilding their panel nodes or cached views. The
    /// floating root is then attached as, or merged into, a main-dock Split(X).
    fn dock_floating_composition_to_main_column(
        &mut self,
        floating_group_id: FloatingGroupId,
        root: NodeId,
        split: Split,
        side: DockSide,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if split.axis != Axis::X || split.children.is_empty() {
            return Err(CommandError::InvalidTarget {
                reason: "floating main-dock composition must use a non-empty horizontal Split",
            });
        }

        let floating = self.floating_groups.get(floating_group_id).copied().ok_or(
            CommandError::MissingFloatingGroup {
                group: floating_group_id,
            },
        )?;
        let source_ratios = normalized_ratios(split.ratios.into_iter().map(Some).collect());
        let mut source_columns = Vec::with_capacity(split.children.len());
        for child in split.children {
            let column = match self.nodes.get(child).cloned() {
                Some(Node::Column(_)) => child,
                Some(_) => {
                    // Generic floating roots may be horizontal splits of Tabs
                    // or nested groups rather than pre-existing Columns. Turn
                    // each branch into one main-dock Column without rebuilding
                    // any panel node or its cached view.
                    let group = self.group_node_for(child)?;
                    self.nodes.insert(Node::Column(crate::Column {
                        width: metrics.default_column_width,
                        mode: ColumnMode::Expanded,
                        groups: vec![group],
                        group_ratios: vec![1.0],
                        last_expanded_width: Some(metrics.default_column_width),
                    }))
                }
                None => return Err(CommandError::MissingNode { node: child }),
            };
            source_columns.push(column);
        }
        let side_slot = side_index(side);
        let Some(main_area) = self.main_area(side).copied() else {
            if index != 0 {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "main-column insertion index is outside the empty dock",
                });
            }
            self.detach_node(root)?;
            let Some(Node::Split(source_split)) = self.nodes.get_mut(root) else {
                return Err(CommandError::InvalidTarget {
                    reason: "floating composition root changed unexpectedly",
                });
            };
            source_split.children = source_columns;
            source_split.ratios = source_ratios;
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

    fn column_composition_stack(
        &self,
        root: NodeId,
    ) -> Result<(Split, Vec<NodeId>), CommandError> {
        match self.nodes.get(root).cloned() {
            Some(Node::Column(column)) => {
                if column.groups.is_empty() || column.groups.len() != column.group_ratios.len() {
                    return Err(CommandError::InvalidTarget {
                        reason: "source Column has an invalid Group stack",
                    });
                }
                Ok((
                    Split {
                        axis: Axis::Y,
                        children: column.groups,
                        ratios: normalized_ratios(
                            column.group_ratios.into_iter().map(Some).collect(),
                        ),
                    },
                    Vec::new(),
                ))
            }
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split.children.iter().all(|child| {
                        matches!(self.nodes.get(*child), Some(Node::Column(_)))
                    }) =>
            {
                let column_ratios =
                    normalized_ratios(split.ratios.into_iter().map(Some).collect());
                let source_columns = split.children.clone();
                let mut groups = Vec::new();
                let mut ratios = Vec::new();

                for (column_index, column_id) in source_columns.iter().copied().enumerate() {
                    let Some(Node::Column(column)) = self.nodes.get(column_id) else {
                        return Err(CommandError::InvalidTarget {
                            reason: "composition contains a missing Column",
                        });
                    };
                    if column.groups.is_empty() || column.groups.len() != column.group_ratios.len() {
                        return Err(CommandError::InvalidTarget {
                            reason: "composition Column has an invalid Group stack",
                        });
                    }
                    let group_ratios = normalized_ratios(
                        column.group_ratios.iter().copied().map(Some).collect(),
                    );
                    for (group_index, group) in column.groups.iter().copied().enumerate() {
                        groups.push(group);
                        ratios.push(column_ratios[column_index] * group_ratios[group_index]);
                    }
                }

                if groups.is_empty() {
                    return Err(CommandError::InvalidTarget {
                        reason: "composition does not contain any Groups",
                    });
                }
                Ok((
                    Split {
                        axis: Axis::Y,
                        children: groups,
                        ratios: normalized_ratios(ratios.into_iter().map(Some).collect()),
                    },
                    source_columns,
                ))
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "source must be a Column or horizontal composition of Columns",
            }),
        }
    }

    fn dock_floating_composition_to_group_edge(
        &mut self,
        root: NodeId,
        target_group: NodeId,
        zone: DropZone,
    ) -> Result<(), CommandError> {
        let target_column = self.find_owner_column(target_group).ok_or(
            CommandError::InvalidTarget {
                reason: "vertical composition target must belong to a Column",
            },
        )?;
        let target_index = match self.nodes.get(target_column) {
            Some(Node::Column(column)) => column
                .groups
                .iter()
                .position(|group| *group == target_group)
                .ok_or(CommandError::InvalidTarget {
                    reason: "target Group is absent from its Column",
                })?,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "vertical composition target must belong to a Column",
                });
            }
        };
        let insert_at = target_index + usize::from(zone == DropZone::Bottom);
        let (stack, source_columns) = self.column_composition_stack(root)?;

        self.dock_floating_stack_to_column_slot(root, stack, target_column, insert_at)?;
        // A Split(X) source contributes its Groups, not its old Column wrappers.
        for source_column in source_columns {
            self.nodes.remove(source_column);
        }
        Ok(())
    }

    pub(super) fn dock_floating_column_to_column_slot(
        &mut self,
        root: NodeId,
        column: Column,
        target_column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        let split = Split {
            axis: Axis::Y,
            children: column.groups,
            ratios: column.group_ratios,
        };
        self.dock_floating_stack_to_column_slot(root, split, target_column, index)
    }

    pub(super) fn dock_floating_column_to_rail_slot(
        &mut self,
        root: NodeId,
        column: Column,
        target_column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        let split = Split {
            axis: Axis::Y,
            children: column.groups,
            ratios: column.group_ratios,
        };
        self.dock_floating_stack_to_rail_slot(root, split, target_column, index)
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

        let target = match self.nodes.get(column) {
            Some(Node::Column(value)) => value.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "column slot target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column }),
        };
        if index > target.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "column group insertion index is outside the column",
            });
        }

        // The incoming floating stack and the existing column each retain half
        // of the available vertical space. Preserve the internal proportions of
        // both layouts instead of assigning every incoming group an equal share.
        let target_ratios = normalized_ratios(
            target
                .groups
                .iter()
                .enumerate()
                .map(|(group_index, _)| target.group_ratios.get(group_index).copied())
                .collect(),
        );
        let source_ratios = normalized_ratios(split.ratios.into_iter().map(Some).collect());

        let mut groups = target.groups;
        groups.splice(index..index, split.children);
        let mut ratios = Vec::with_capacity(groups.len());
        ratios.extend(target_ratios[..index].iter().map(|ratio| ratio * 0.5));
        ratios.extend(source_ratios.iter().map(|ratio| ratio * 0.5));
        ratios.extend(target_ratios[index..].iter().map(|ratio| ratio * 0.5));
        let ratios = normalized_ratios(ratios.into_iter().map(Some).collect());

        self.detach_node(root)?;
        self.nodes.remove(root);
        let Some(Node::Column(target)) = self.nodes.get_mut(column) else {
            return Err(CommandError::InvalidTarget {
                reason: "column slot target changed unexpectedly",
            });
        };
        target.groups = groups;
        target.group_ratios = ratios;
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
    /// floating areas are reserved for joining complete Column-based compositions;
    /// the general GroupEdge validator remains scoped to main-dock splits.
    pub(super) fn is_column_composition_root(&self, root: NodeId) -> bool {
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

    pub(super) fn validate_floating_dock_target(
        &self,
        source_root: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let source_is_column_composition = self.is_column_composition_root(source_root);
        if let DropTarget::GroupEdge {
            group,
            zone: zone @ (
                DropZone::Left | DropZone::Right | DropZone::FloatingColumnInsertion
            ),
        } = target
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
        if !matches!(
            zone,
            DropZone::Left | DropZone::Right | DropZone::FloatingColumnInsertion
        ) {
            return Err(CommandError::InvalidTarget {
                reason: "floating composition docking requires an outer edge or internal column splitter",
            });
        }
        if self.is_group_in_icon_rail(group) {
            return Err(CommandError::InvalidTarget {
                reason: "floating composition edge target cannot be an icon rail slot",
            });
        }
        if zone == DropZone::FloatingColumnInsertion {
            let Some(column) = self.find_owner_column(group) else {
                return Err(CommandError::InvalidTarget {
                    reason: "Floating column insertion target must belong to a Column",
                });
            };
            let Some(Node::Column(column_data)) = self.nodes.get(column) else {
                return Err(CommandError::InvalidTarget {
                    reason: "Floating column insertion target must belong to a Column",
                });
            };
            if column_data.mode.is_rail() || column_data.groups.first() != Some(&group) {
                return Err(CommandError::InvalidTarget {
                    reason: "Floating column insertion must target the first Group in an expanded Column",
                });
            }
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
        let is_valid_boundary = match self.nodes.get(root) {
            Some(Node::Column(_)) => {
                root == column && matches!(zone, DropZone::Left | DropZone::Right)
            },
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
                match zone {
                    DropZone::Left => index == 0,
                    DropZone::Right => index + 1 == split.children.len(),
                    DropZone::FloatingColumnInsertion => {
                        index > 0
                            && !matches!(
                                split.children.get(index - 1).and_then(|id| self.nodes.get(*id)),
                                Some(Node::Column(previous)) if previous.mode.is_rail()
                            )
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !is_valid_boundary {
            return Err(CommandError::InvalidTarget {
                reason: "floating horizontal docking requires an outer edge or internal Column splitter",
            });
        }
        Ok(())
    }

    /// Joins two Floating areas side-by-side while retaining each area's
    /// Columns as independent children of one horizontal composition.
    ///
    /// The destination FloatingGroup and its DockArea remain the window host;
    /// the source FloatingGroup is removed when its root is detached.
    fn dock_floating_composition_to_edge(
        &mut self,
        source_group_id: FloatingGroupId,
        source_root: NodeId,
        target_group: NodeId,
        zone: DropZone,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.floating_groups
            .get(source_group_id)
            .ok_or(CommandError::MissingFloatingGroup {
                group: source_group_id,
            })?;
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

        let (source_columns, source_was_split) = match self.nodes.get(source_root) {
            Some(Node::Column(_)) => (vec![source_root], false),
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                (split.children.clone(), true)
            }
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "source floating area must be a Column or horizontal composition of Columns",
                });
            }
        };
        let (target_columns, target_was_split) = match self.nodes.get(target_root) {
            Some(Node::Column(_)) => (vec![target_root], false),
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                (split.children.clone(), true)
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
        let insertion_index = match zone {
            DropZone::Left if target_index == 0 => 0,
            DropZone::Right if target_index + 1 == target_columns.len() => {
                target_columns.len()
            }
            DropZone::FloatingColumnInsertion if target_index > 0 => target_index,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "floating composition must target an outer edge or an internal column splitter",
                });
            }
        };

        let mut children = Vec::with_capacity(source_columns.len() + target_columns.len());
        for (index, target_column) in target_columns.iter().copied().enumerate() {
            if index == insertion_index {
                children.extend(source_columns.iter().copied());
            }
            children.push(target_column);
        }
        if insertion_index == target_columns.len() {
            children.extend(source_columns.iter().copied());
        }

        // Floating compositions fill their native client area. When two
        // windows are joined, preserve the widths currently visible in both
        // windows rather than reverting to each Column's saved preferred width.
        let source_rects = self
            .compute_floating_rects(source_group_id, metrics)
            .map_err(|_| CommandError::InvalidTarget {
                reason: "source Floating composition geometry could not be computed",
            })?;
        let target_rects = self
            .compute_floating_rects(target_group_id, metrics)
            .map_err(|_| CommandError::InvalidTarget {
                reason: "target Floating composition geometry could not be computed",
            })?;

        let column_widths = children
            .iter()
            .map(|column_id| {
                source_rects
                    .columns
                    .iter()
                    .chain(target_rects.columns.iter())
                    .find(|column| column.column == *column_id)
                    .map(|column| column.rect.size.width)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "Floating composition is missing a current Column rectangle",
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let total_column_width = column_widths.iter().sum::<f32>();
        let splitter_width = metrics.splitter_thickness
            * children.len().saturating_sub(1) as f32;
        let required_width = total_column_width + splitter_width;
        let source_column_widths = source_columns
            .iter()
            .map(|column_id| {
                source_rects
                    .columns
                    .iter()
                    .find(|column| column.column == *column_id)
                    .map(|column| column.rect.size.width)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "source Floating composition is missing a current Column rectangle",
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let source_composition_width = source_column_widths.iter().sum::<f32>()
            + metrics.splitter_thickness
                * source_columns.len().saturating_sub(1) as f32;
        if !total_column_width.is_finite()
            || total_column_width <= 0.0
            || !required_width.is_finite()
            || !source_composition_width.is_finite()
            || source_composition_width <= 0.0
        {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "combined floating composition width must be finite and positive",
            });
        }
        let window_width = required_width;
        let ratios = normalized_ratios(column_widths.into_iter().map(Some).collect());

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

        // Keep the outer edge opposite the insertion side anchored. This lets
        // a left-side drop grow the destination window leftward while a right
        // or internal insertion grows it to the right.
        let target_origin_x = if insertion_index == 0 {
            target_floating.bounds.origin.x
                - source_composition_width
                - metrics.splitter_thickness
        } else {
            target_floating.bounds.origin.x
        };
        if !target_origin_x.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "origin",
                reason: "combined floating composition origin must be finite",
            });
        }
        let Some(target) = self.floating_groups.get_mut(target_group_id) else {
            return Err(CommandError::MissingFloatingGroup {
                group: target_group_id,
            });
        };
        target.bounds = Rect::new(
            Point::new(target_origin_x, target_floating.bounds.origin.y),
            Size::new(window_width, target_floating.bounds.size.height),
        );

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

    /// Compatibility wrapper for callers that still report the native edge.
    ///
    /// Floating compositions always fill the whole client area, so the edge
    /// does not select or resize an individual Column. The native window's
    /// bounds are the only input to this operation.
    pub(super) fn apply_resize_floating_bounds_from_edge_command(
        &mut self,
        group: FloatingGroupId,
        bounds: Rect,
        _edge: crate::FloatingResizeEdge,
        scale_factor: f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "scale_factor",
                reason: "floating resize scale factor must be finite and positive",
            });
        }
        self.apply_resize_floating_bounds_command(group, bounds, metrics)
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
