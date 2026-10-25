use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn float_group_as_column(
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
}
