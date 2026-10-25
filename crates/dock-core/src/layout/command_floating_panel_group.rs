use super::*;
use super::helpers::geometry_error_reason;
use crate::AllowedArea;

impl Layout {
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
