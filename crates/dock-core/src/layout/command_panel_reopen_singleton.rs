use super::*;

impl Layout {
    pub(super) fn try_reopen_existing_singleton_panel(
        &mut self,
        key: &PanelKey,
        requested_singleton: bool,
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some((existing_id, _)) = self.panel_instances.iter().find(|(_, instance)| {
            instance.key == *key && (requested_singleton || instance.constraints.singleton)
        }) else {
            return Ok(None);
        };

        let panel_node = self
            .find_panel_node(existing_id)
            .ok_or(CommandError::MissingPanel { panel: existing_id })?;
        let Some((group, index)) = self.tab_parent_index(panel_node) else {
            return Ok(Some(CommandApplication::persistent()));
        };
        let tabs = match self.nodes.get(group) {
            Some(Node::Tabs(tabs)) => tabs,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "existing singleton panel parent is not a Tabs node",
                });
            }
        };
        let previous_active = tabs.children.get(tabs.active_index).and_then(|id| {
            match self.nodes.get(*id) {
                Some(Node::Panel(panel)) => Some(panel.panel_id),
                _ => None,
            }
        });
        if !tabs.children.contains(&panel_node) {
            return Err(CommandError::InvalidTarget {
                reason: "existing singleton panel is not a child of its Tabs node",
            });
        }
        let was_active = index == tabs.active_index;
        if !was_active && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) {
            tabs.active_index = index;
        }
        let previous_flyout = self.flyout;
        if let Some(column) = self.rail_column_for_group(group) {
            self.flyout = Some(crate::FlyoutState::new(column, group, existing_id));
        }
        if !was_active {
            return Ok(Some(CommandApplication {
                actual_value: None,
                inverse: previous_active
                    .map(|panel| DockCommand::ActivateTab { group, panel }),
                persistent: true,
            }));
        }
        if self.flyout != previous_flyout {
            return Ok(Some(CommandApplication::transient()));
        }
        Ok(Some(CommandApplication::persistent()))
    }
}
