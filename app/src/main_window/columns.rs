use dock_core::{Axis, DockCommand, DockSide, DropTarget, DropZone, Node, PanelKey};

use super::{group_ids_for_column, ClosedColumnSnapshot, ClosedTabGroupSnapshot, MainWindow};

impl MainWindow {
    fn open_panel_for_restore(
        &mut self,
        panel_key: &str,
        target: Option<DropTarget>,
    ) -> Result<(), String> {
        let key = PanelKey::new(panel_key)
            .map_err(|_| format!("Invalid panel key while restoring column: {panel_key}"))?;
        let constraints = self
            .panels
            .constraints_for_key(&key)
            .ok_or_else(|| format!("Panel registry has no descriptor for '{panel_key}'"))?;
        self.layout
            .apply(
                DockCommand::OpenPanel {
                    key,
                    constraints,
                    target,
                },
                &self.dock_metrics,
            )
            .map_err(|error| format!("Could not recreate panel '{panel_key}': {error}"))?;
        Ok(())
    }

    pub(super) fn open_column_from_menu(
        &mut self,
        column_index: usize,
        selected_panel_key: &str,
    ) -> Result<(), String> {
        if column_index >= self.column_ids.len()
            || column_index >= self.column_open.len()
            || column_index >= self.closed_column_snapshots.len()
        {
            return Err("Cannot restore a dock column at an unknown column slot".to_owned());
        }

        let mut snapshot = self
            .closed_column_snapshots
            .get(column_index)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| ClosedColumnSnapshot {
                position: column_index,
                groups: Vec::new(),
                group_ratios: Vec::new(),
                width: self.theme.metrics.default_column_width,
            });
        snapshot.groups.retain(|group| !group.panel_keys.is_empty());
        if snapshot.groups.is_empty() {
            snapshot.groups.push(ClosedTabGroupSnapshot {
                panel_keys: vec![selected_panel_key.to_owned()],
                active_panel_key: Some(selected_panel_key.to_owned()),
                collapsed: false,
            });
            snapshot.group_ratios = vec![1.0];
        } else if !snapshot
            .groups
            .iter()
            .any(|group| group.panel_keys.iter().any(|key| key == selected_panel_key))
        {
            // A menu entry may have been discovered after this column was closed.
            // Put an unknown-to-the-snapshot panel into the first restored group.
            snapshot.groups[0]
                .panel_keys
                .push(selected_panel_key.to_owned());
        }

        let first_panel_key = snapshot.groups[0].panel_keys[0].clone();
        // Restore at the visual position captured before closing. Stable UI slot
        // numbers are not authoritative after Core reorders the columns.
        let main_column_count = self
            .layout
            .main_area(DockSide::Right)
            .map(|area| self.core_column_ids_in_order(area.root).len())
            .unwrap_or_default();
        let insert_index = snapshot.position.min(main_column_count);
        let target = (main_column_count > 0).then_some(DropTarget::MainColumn {
            side: DockSide::Right,
            index: insert_index,
        });
        self.open_panel_for_restore(&first_panel_key, target)?;

        let area = self
            .layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core did not create a main dock area".to_owned())?;
        let target_column = match self.layout.node(area.root) {
            Some(Node::Split(split)) if split.axis == Axis::X && !split.children.is_empty() => {
                split
                    .children
                    .get(insert_index)
                    .copied()
                    .ok_or_else(|| "Restored column is missing from the Core split".to_owned())?
            }
            Some(Node::Column(_)) => area.root,
            _ => return Err("Dock Core created an unexpected main dock shape".to_owned()),
        };
        // Preserve closed slots and their snapshots; only the restored slot gets
        // a new Core node ID and changes back to open.
        self.column_ids[column_index] = target_column;
        self.column_open[column_index] = true;

        let first_group = group_ids_for_column(&self.layout, target_column)
            .first()
            .copied()
            .ok_or_else(|| "Dock Core did not create the first restored tab group".to_owned())?;
        let mut restored_groups = Vec::with_capacity(snapshot.groups.len());
        restored_groups.push(first_group);

        for (group_index, group_snapshot) in snapshot.groups.iter().enumerate() {
            let group_id = if group_index == 0 {
                first_group
            } else {
                let first_key = group_snapshot
                    .panel_keys
                    .first()
                    .ok_or_else(|| "A restored tab group has no panels".to_owned())?;
                let previous_group = *restored_groups
                    .last()
                    .ok_or_else(|| "No preceding group exists during restore".to_owned())?;
                let groups_before = self.group_ids_in_column(target_column);
                self.open_panel_for_restore(
                    first_key,
                    Some(DropTarget::GroupEdge {
                        group: previous_group,
                        zone: DropZone::Bottom,
                    }),
                )?;
                let groups_after = self.group_ids_in_column(target_column);
                let new_group = groups_after
                    .iter()
                    .copied()
                    .find(|group| !groups_before.contains(group))
                    .ok_or_else(|| "Dock Core did not create a new restored tab group".to_owned())?;
                restored_groups.push(new_group);
                new_group
            };

            for panel_key in group_snapshot.panel_keys.iter().skip(1) {
                let target_index = self
                    .tab_group_view_data(group_id)
                    .map_or(0, |group| group.panels.len());
                self.open_panel_for_restore(
                    panel_key,
                    Some(DropTarget::Tabs {
                        group: group_id,
                        index: target_index,
                    }),
                )?;
            }
        }

        for (group_snapshot, group_id) in snapshot.groups.iter().zip(restored_groups.iter().copied()) {
            let selected_key_for_group = group_snapshot
                .panel_keys
                .iter()
                .any(|key| key == selected_panel_key)
                .then_some(selected_panel_key);
            let active_key = selected_key_for_group.or(group_snapshot.active_panel_key.as_deref());
            if let Some(active_key) = active_key {
                let active_panel = self
                    .tab_group_view_data(group_id)
                    .into_iter()
                    .flat_map(|group| group.panels)
                    .find(|panel_id| {
                        self.layout
                            .panel_instance(*panel_id)
                            .is_some_and(|panel| panel.key.as_str() == active_key)
                    });
                if let Some(active_panel) = active_panel {
                    self.layout
                        .apply(
                            DockCommand::ActivateTab {
                                group: group_id,
                                panel: active_panel,
                            },
                            &self.dock_metrics,
                        )
                        .map_err(|error| format!("Could not restore active panel: {error}"))?;
                }
            }
            if self.group_is_collapsed(group_id) != group_snapshot.collapsed {
                self.layout
                    .apply(
                        DockCommand::CollapseGroup {
                            group: group_id,
                            collapsed: group_snapshot.collapsed,
                        },
                        &self.dock_metrics,
                    )
                    .map_err(|error| format!("Could not restore group collapse state: {error}"))?;
            }
        }

        let current_groups = self.group_ids_in_column(target_column);
        if snapshot.group_ratios.len() == current_groups.len() {
            self.layout
                .apply(
                    DockCommand::RestoreGroupRatios {
                        column: target_column,
                        ratios: snapshot.group_ratios,
                    },
                    &self.dock_metrics,
                )
                .map_err(|error| format!("Could not restore group sizes: {error}"))?;
        }
        if snapshot.width.is_finite() && snapshot.width > 0.0 {
            self.layout
                .apply(
                    DockCommand::ResizeColumn {
                        column: target_column,
                        width: snapshot.width,
                    },
                    &self.dock_metrics,
                )
                .map_err(|error| format!("Could not restore column width: {error}"))?;
        }

        self.closed_column_snapshots[column_index] = None;
        self.sync_floating_composition_bounds();
        self.sync_panel_visibility_entries();
        Ok(())
    }

    fn snapshot_column(&self, column_index: usize) -> ClosedColumnSnapshot {
        let column_id = self.column_ids[column_index];
        let groups = self
            .group_ids_in_column(column_id)
            .into_iter()
            .filter_map(|group_id| {
                let group = self.tab_group_view_data(group_id)?;
                let panel_keys: Vec<String> = group
                    .panels
                    .iter()
                    .filter_map(|panel_id| {
                        self.layout
                            .panel_instance(*panel_id)
                            .map(|panel| panel.key.as_str().to_owned())
                    })
                    .collect();
                if panel_keys.is_empty() {
                    return None;
                }
                let active_panel_key = group.active_panel.and_then(|panel_id| {
                    self.layout
                        .panel_instance(panel_id)
                        .map(|panel| panel.key.as_str().to_owned())
                });
                Some(ClosedTabGroupSnapshot {
                    panel_keys,
                    active_panel_key,
                    collapsed: group.collapsed,
                })
            })
            .collect();

        ClosedColumnSnapshot {
            position: self.column_visual_position(column_index),
            groups,
            group_ratios: self.group_ratios_in_column(column_id),
            width: self.column_width(column_index),
        }
    }

    pub(super) fn close_column(&mut self, column_index: usize) {
        if column_index >= self.column_open.len() || !self.column_open[column_index] {
            return;
        }
        // Save the actual Core group structure before close commands normalize it away.
        let snapshot = self.snapshot_column(column_index);
        // Close every group in the Core column, without assuming a top/bottom pair.
        // Take a snapshot because each close command may normalize Core.
        let column_id = self.column_ids[column_index];
        let groups = self.group_ids_in_column(column_id);
        for group in groups {
            if self.layout.node(group).is_none() {
                continue;
            }
            if let Err(error) = self.apply_dock_command(
                DockCommand::CloseGroup { group }) {
                eprintln!("Could not close dock group: {error}");
                return;
            }
        }

        self.closed_column_snapshots[column_index] = Some(snapshot);

        // Core normalization removes the empty Column and updates its parent Split.
        // Rendering follows that normalized topology on the next frame.
        self.column_open[column_index] = false;
        for entry in &mut self.panel_visibility {
            if entry.column_index == column_index {
                entry.visible = false;
            }
        }
        self.is_resizing_column = false;
        self.resizing_outer_column = None;
        self.resizing_groups_column = None;
    }

    pub(super) fn column_width(&self, column_index: usize) -> f32 {
        self.layout
            .node(self.column_ids[column_index])
            .and_then(|node| match node {
                Node::Column(column) => Some(column.width),
                _ => None,
            })
            .unwrap_or(self.theme.metrics.default_column_width)
    }
}
