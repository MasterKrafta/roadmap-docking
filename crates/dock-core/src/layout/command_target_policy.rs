use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn allowed_area_for_target(&self, target: DropTarget) -> Result<AllowedArea, CommandError> {
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

    fn allowed_area_for_group_target(&self, group: NodeId) -> Result<AllowedArea, CommandError> {
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
                Some(Node::Column(column)) if column.mode.is_rail() => Ok(AllowedArea::ICON_RAIL),
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

    pub(super) fn is_group_in_icon_rail(&self, group: NodeId) -> bool {
        let Some(column_id) = self.find_owner_column(group) else {
            return false;
        };
        self.main_column_position(column_id).is_some()
            && matches!(self.nodes.get(column_id), Some(Node::Column(column)) if column.mode.is_rail())
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
}
