use super::*;
use super::helpers::{equal_ratios, normalized_ratios, side_index, split_sibling_ratios};
use crate::AllowedArea;

impl Layout {
    pub(super) fn allowed_area_for_column_target(
        &self,
        column_id: NodeId,
    ) -> Result<AllowedArea, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group destination must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };

        if self.find_floating_root_containing(column_id).is_some() {
            return Ok(AllowedArea::FLOATING);
        }

        for side in [DockSide::Left, DockSide::Right] {
            if self
                .main_area(side)
                .is_some_and(|area| self.is_descendant_or_same(area.root, column_id))
            {
                return Ok(if column.mode.is_rail() {
                    AllowedArea::ICON_RAIL
                } else {
                    AllowedArea::MAIN_DOCK
                });
            }
        }

        Err(CommandError::InvalidTarget {
            reason: "group destination column is not attached to a main or floating dock area",
        })
    }

    pub(super) fn allowed_area_for_group_target(
        &self,
        group: NodeId,
    ) -> Result<AllowedArea, CommandError> {
        if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
            return Err(if self.nodes.contains_key(group) {
                CommandError::InvalidTarget {
                    reason: "drop group target must be a Tabs node",
                }
            } else {
                CommandError::MissingNode { node: group }
            });
        }

        // Floating membership takes precedence because Generic layouts may contain
        // arbitrary nested Split trees inside a floating area.
        if self.find_floating_root_containing(group).is_some() {
            return Ok(AllowedArea::FLOATING);
        }

        if let Some(column_id) = self.find_owner_column(group)
            && self.main_column_position(column_id).is_some()
        {
            return match self.nodes.get(column_id) {
                Some(Node::Column(column)) if column.mode.is_rail() => {
                    Ok(AllowedArea::ICON_RAIL)
                }
                Some(Node::Column(_)) => Ok(AllowedArea::MAIN_DOCK),
                _ => Err(CommandError::InvalidTarget {
                    reason: "drop group owner is not a Column",
                }),
            };
        }

        for side in [DockSide::Left, DockSide::Right] {
            if self
                .main_area(side)
                .is_some_and(|area| self.is_descendant_or_same(area.root, group))
            {
                return Ok(AllowedArea::MAIN_DOCK);
            }
        }

        Err(CommandError::InvalidTarget {
            reason: "drop target is not attached to a main or floating dock area",
        })
    }

    pub(super) fn allowed_area_for_target(
        &self,
        target: DropTarget,
    ) -> Result<AllowedArea, CommandError> {
        match target {
            DropTarget::Tabs { group, .. }
            | DropTarget::GroupTabs { group, .. }
            | DropTarget::GroupEdge { group, .. } => self.allowed_area_for_group_target(group),
            DropTarget::MainColumn { .. } | DropTarget::ColumnSlot { .. } => {
                Ok(AllowedArea::MAIN_DOCK)
            }
            DropTarget::RailSlot { .. } => Ok(AllowedArea::ICON_RAIL),
            DropTarget::FreeArea => Ok(AllowedArea::FLOATING),
        }
    }

    pub(super) fn validate_panel_ids_allowed_for_area(
        &self,
        panels: &[PanelId],
        area: AllowedArea,
    ) -> Result<(), CommandError> {
        for panel in panels {
            let instance = self
                .panel_instance(*panel)
                .ok_or(CommandError::MissingPanel { panel: *panel })?;
            if !instance.constraints.allows_area(area) {
                return Err(CommandError::InvalidTarget {
                    reason: allowed_area_rejection_reason(area),
                });
            }
        }
        Ok(())
    }

    pub(super) fn validate_constraints_allowed_for_target(
        &self,
        constraints: crate::PanelConstraints,
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        if constraints.allows_area(area) {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: allowed_area_rejection_reason(area),
            })
        }
    }

    pub(super) fn validate_panel_ids_allowed_for_target(
        &self,
        panels: &[PanelId],
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        self.validate_panel_ids_allowed_for_area(panels, area)
    }

    pub(super) fn validate_drop_target(
        &self,
        target: DropTarget,
        _metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        match target {
            DropTarget::Tabs { group, index } => match self.nodes.get(group) {
                Some(Node::Tabs(_)) if self.is_group_in_icon_rail(group) => {
                    Err(CommandError::InvalidTarget {
                        reason: "tab insertion target is an icon rail slot; use RailSlot",
                    })
                }
                Some(Node::Tabs(tabs)) if index <= tabs.children.len() => Ok(()),
                Some(Node::Tabs(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "tab insertion index is outside the group",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "tab target must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            },
            DropTarget::GroupTabs { group, index } => match self.nodes.get(group) {
                Some(Node::Tabs(_)) if self.is_group_in_icon_rail(group) => {
                    Err(CommandError::InvalidTarget {
                        reason: "group-merge target is an icon rail slot; use RailSlot",
                    })
                }
                Some(Node::Tabs(tabs)) if index <= tabs.children.len() => Ok(()),
                Some(Node::Tabs(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "group-tab insertion index is outside the group",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "group-tab target must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            },
            DropTarget::GroupEdge { group, zone } => {
                if !matches!(
                    zone,
                    DropZone::Left | DropZone::Right | DropZone::Top | DropZone::Bottom
                ) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group-edge target requires a cardinal edge zone",
                    });
                }
                if self.is_group_in_icon_rail(group) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group-edge target is an icon rail slot; use RailSlot",
                    });
                }
                match self.nodes.get(group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "group-edge target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: group }),
                }
                if self.profile == Profile::Photoshop {
                    match zone {
                        DropZone::Left | DropZone::Right => {
                            let Some(column) = self.find_owner_column(group) else {
                                return Err(CommandError::InvalidTarget {
                                    reason: if self.find_floating_root_containing(group).is_some() {
                                        "horizontal splits are not supported inside a floating stack"
                                    } else {
                                        "horizontal group splits require a main-dock column"
                                    },
                                });
                            };
                            if self.main_column_position(column).is_none() {
                                return Err(CommandError::InvalidTarget {
                                    reason: "horizontal group splits require a column in a main dock",
                                });
                            }
                        }
                        DropZone::Top | DropZone::Bottom => {
                            if self.find_owner_column(group).is_none()
                                && self.find_floating_root_containing(group).is_none()
                            {
                                return Err(CommandError::InvalidTarget {
                                    reason: "vertical group splits require a column or floating stack",
                                });
                            }
                        }
                        _ => {
                            return Err(CommandError::InvalidTarget {
                                reason: "group-edge target requires a cardinal edge zone",
                            });
                        }
                    }
                }
                Ok(())
            }
            DropTarget::MainColumn { side, index } => {
                let columns = self.main_columns(side)?;
                if index <= columns.len() {
                    Ok(())
                } else {
                    Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "main column insertion index is outside the dock",
                    })
                }
            }
            DropTarget::ColumnSlot { column, index } => match self.nodes.get(column) {
                Some(Node::Column(value)) if value.mode.is_rail() => {
                    Err(CommandError::InvalidTarget {
                        reason: "expanded-column insertion target cannot be a rail; use RailSlot",
                    })
                }
                Some(Node::Column(value)) if self.main_column_position(column).is_none() => {
                    let _ = value;
                    Err(CommandError::InvalidTarget {
                        reason: "column insertion target must belong to a main dock",
                    })
                }
                Some(Node::Column(value)) if index <= value.groups.len() => Ok(()),
                Some(Node::Column(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "column group insertion index is outside the column",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "column slot target must be a Column node",
                }),
                None => Err(CommandError::MissingNode { node: column }),
            },
            DropTarget::FreeArea => Err(CommandError::InvalidTarget {
                reason: "FreeArea must be resolved by Float with explicit window bounds",
            }),
            DropTarget::RailSlot { column, index } => match self.nodes.get(column) {
                Some(Node::Column(value)) if !value.mode.is_rail() => {
                    Err(CommandError::InvalidTarget {
                        reason: "rail insertion target must use a rail mode",
                    })
                }
                Some(Node::Column(_)) if self.main_column_position(column).is_none() => {
                    Err(CommandError::InvalidTarget {
                        reason: "rail insertion target must belong to a main dock column",
                    })
                }
                Some(Node::Column(_)) => {
                    let count = self.column_display_order(column)?.len();
                    if index <= count {
                        Ok(())
                    } else {
                        Err(CommandError::InvalidValue {
                            field: "index",
                            reason: "rail insertion index is outside the flattened icon order",
                        })
                    }
                }
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "rail target must be a Column node",
                }),
                None => Err(CommandError::MissingNode { node: column }),
            },
        }
    }

    pub(super) fn insert_node_for_target(
        &mut self,
        node: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
        edge_ratio: Option<f32>,
    ) -> Result<(), CommandError> {
        self.validate_drop_target(target, metrics)?;
        match target {
            DropTarget::Tabs { group, index } => {
                if !matches!(self.nodes.get(node), Some(Node::Panel(_))) {
                    return Err(CommandError::InvalidTarget {
                        reason: "Tabs target accepts a Panel leaf; move a group to a group or column target",
                    });
                }
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "tab target changed unexpectedly",
                    });
                };
                tabs.children.insert(index, node);
                tabs.active_index = index;
                Ok(())
            }
            DropTarget::GroupTabs { .. } => Err(CommandError::InvalidTarget {
                reason: "whole Tabs groups must be merged through MergeGroup",
            }),
            DropTarget::GroupEdge { group, zone } => {
                let group_node = self.group_node_for(node)?;
                self.insert_group_edge(group_node, group, zone, edge_ratio, metrics)
            }
            DropTarget::MainColumn { side, index } => {
                let group = self.group_node_for(node)?;
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(side, index, column, None, metrics)
            }
            DropTarget::ColumnSlot { column, index } => {
                let group = self.group_node_for(node)?;
                self.insert_group_at_column(column, group, index, None)
            }
            DropTarget::RailSlot { column, index } => {
                if matches!(self.nodes.get(node), Some(Node::Panel(_))) {
                    self.insert_panel_for_rail_slot(node, column, index, metrics)?;
                    Ok(())
                } else {
                    let group_index = self.rail_slot_group_index(column, index)?;
                    let group = self.group_node_for(node)?;
                    self.insert_group_at_column(column, group, group_index, None)
                }
            }
            DropTarget::FreeArea => Err(CommandError::InvalidTarget {
                reason: "FreeArea must be resolved by Float with explicit window bounds",
            }),
        }
    }

    pub(super) fn group_node_for(&mut self, node: NodeId) -> Result<NodeId, CommandError> {
        match self.nodes.get(node).cloned() {
            Some(Node::Tabs(_)) => Ok(node),
            Some(Node::Panel(_)) => Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![node],
                active_index: 0,
                collapsed: false,
            }))),
            Some(_) if self.profile == Profile::Generic => {
                Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                    children: vec![node],
                    active_index: 0,
                    collapsed: false,
                })))
            }
            Some(_) => Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "Photoshop columns and floating stacks can contain Tabs groups or Panel leaves only",
            }),
            None => Err(CommandError::MissingNode { node }),
        }
    }

    pub(super) fn insert_group_at_column(
        &mut self,
        column_id: NodeId,
        group_id: NodeId,
        index: usize,
        split_ratio: Option<(f32, usize)>,
    ) -> Result<(), CommandError> {
        if group_id == column_id {
            return Err(CommandError::InvalidTarget {
                reason: "a column cannot contain itself",
            });
        }
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group insertion target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if index > column.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "group insertion index is outside the column",
            });
        }
        if column.groups.contains(&group_id) {
            return Err(CommandError::InvalidTarget {
                reason: "group is already in the target column",
            });
        }
        let mut groups = column.groups.clone();
        groups.insert(index, group_id);
        let ratios = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue {
                    field: "ratio",
                    reason: "split ratio must be between zero and one",
                });
            }
            let old_len = column.groups.len();
            if target_index >= old_len {
                return Err(CommandError::InvalidTarget {
                    reason: "split ratio target does not identify an existing group",
                });
            }
            split_sibling_ratios(
                &column.group_ratios,
                old_len,
                target_index,
                index,
                ratio,
            )
        } else if column.groups.is_empty() {
            vec![1.0]
        } else {
            let old_ratios = normalized_ratios(
                column.groups.iter().enumerate().map(|(index, _)| column.group_ratios.get(index).copied()).collect(),
            );
            let old_scale = column.groups.len() as f32 / groups.len() as f32;
            let mut preserved = Vec::with_capacity(groups.len());
            for group_index in 0..groups.len() {
                if group_index == index {
                    preserved.push(1.0 / groups.len() as f32);
                } else {
                    let old_index = if group_index < index { group_index } else { group_index - 1 };
                    preserved.push(old_ratios[old_index] * old_scale);
                }
            }
            normalized_ratios(preserved.into_iter().map(Some).collect())
        };
        let Some(Node::Column(column)) = self.nodes.get_mut(column_id) else {
            return Err(CommandError::InvalidTarget {
                reason: "target column changed unexpectedly",
            });
        };
        column.groups = groups;
        column.group_ratios = ratios;
        Ok(())
    }

    pub(super) fn insert_existing_column_at(
        &mut self,
        side: DockSide,
        index: usize,
        new_column: NodeId,
        split_ratio: Option<(f32, usize)>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if !matches!(self.nodes.get(new_column), Some(Node::Column(_))) {
            return Err(CommandError::InvalidTarget {
                reason: "inserted main column is not a Column node",
            });
        }
        let split_ratio = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue {
                    field: "ratio",
                    reason: "split ratio must be between zero and one",
                });
            }
            Some((ratio, target_index))
        } else {
            None
        };

        let Some(area_id) = self.main_areas[side_index(side)] else {
            if index != 0 {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "first column on an empty dock must use index zero",
                });
            }
            let area_id = self.dock_areas.insert(DockArea::main(side, new_column));
            self.main_areas[side_index(side)] = Some(area_id);
            return Ok(());
        };
        let area_root = self
            .dock_areas
            .get(area_id)
            .ok_or(CommandError::InvalidTarget {
                reason: "main dock side references a missing area",
            })?
            .root;

        match self.nodes.get(area_root).cloned() {
            Some(Node::Column(_)) => {
                if index > 1 {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index != 0 {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target does not identify the existing column",
                        });
                    }
                    self.resize_inserted_column_for_split_ratio(
                        area_root,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                let children = if index == 0 {
                    vec![new_column, area_root]
                } else {
                    vec![area_root, new_column]
                };
                let split = self.nodes.insert(Node::Split(crate::Split {
                    axis: Axis::X,
                    ratios: equal_ratios(2),
                    children,
                }));
                if let Some(area) = self.dock_areas.get_mut(area_id) {
                    area.root = split;
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(Node::Split(mut split)) if split.axis == Axis::X => {
                if index > split.children.len() {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split contains a non-Column child",
                    });
                }
                let old_len = split.children.len();
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index >= old_len {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target or column insertion index is outside the dock",
                        });
                    }
                    let target_column = split.children[target_index];
                    self.resize_inserted_column_for_split_ratio(
                        target_column,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                split.children.insert(index, new_column);
                // Main-column Split(X) ratios mirror visible column widths. They
                // are recalculated after the tree is updated below.
                split.ratios = equal_ratios(old_len + 1);
                if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
                    *current = split;
                } else {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split changed unexpectedly",
                    });
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(_) => Err(CommandError::InvalidTarget {
                reason: "main dock root must be a Column or Split(X) of Columns",
            }),
            None => Err(CommandError::InvalidTarget {
                reason: "main dock root is missing",
            }),
        }
    }
}

pub(super) fn allowed_area_rejection_reason(area: AllowedArea) -> &'static str {
    if area == AllowedArea::MAIN_DOCK {
        "panel constraints disallow the main dock area"
    } else if area == AllowedArea::FLOATING {
        "panel constraints disallow floating areas"
    } else if area == AllowedArea::ICON_RAIL {
        "panel constraints disallow icon rails"
    } else {
        "panel constraints disallow the requested dock area"
    }
}
