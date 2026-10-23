use dock_core::{ColumnMode, DockCommand, NodeId, PanelId};

use super::MainWindow;

#[derive(Clone, Copy)]
pub(super) struct DockTabContextMenu {
    pub(super) column_index: usize,
    pub(super) group: NodeId,
    pub(super) active_panel: Option<PanelId>,
    pub(super) header_only: bool,
    pub(super) left: f32,
    pub(super) top: f32,
}

/// UI visibility state is keyed by the Core panel instance, not by a menu index.
pub(super) struct PanelVisibilityEntry {
    pub(super) panel_id: PanelId,
    pub(super) column_index: usize,
    pub(super) panel_key: String,
    pub(super) title: String,
    pub(super) visible: bool,
}

impl MainWindow {
    pub(super) fn toggle_tab_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        active_panel: Option<PanelId>,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        if self
            .tab_context_menu
            .is_some_and(|menu| menu.group == group && !menu.header_only)
        {
            self.tab_context_menu = None;
            return;
        }
        self.open_tab_context_menu(column_index, group, active_panel, pointer_x, pointer_y);
    }

    fn open_tab_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        active_panel: Option<PanelId>,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        self.windows_menu_open = false;
        self.tab_context_menu = Some(DockTabContextMenu {
            column_index,
            group,
            active_panel,
            header_only: false,
            left: pointer_x.max(0.0),
            top: pointer_y.max(0.0),
        });
    }

    pub(super) fn open_header_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        self.windows_menu_open = false;
        self.tab_context_menu = Some(DockTabContextMenu {
            column_index,
            group,
            active_panel: None,
            header_only: true,
            left: pointer_x.max(0.0),
            top: pointer_y.max(0.0),
        });
    }

    pub(super) fn close_context_tab(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let Some(panel_id) = menu.active_panel else {
            return;
        };
        if self.panel_is_visible(panel_id) {
            self.toggle_panel_visibility(panel_id);
        }
    }

    pub(super) fn close_context_tab_group(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let panel_ids = self
            .tab_group_view_data(menu.group)
            .map(|group| group.panels)
            .unwrap_or_default();
        for panel_id in panel_ids {
            if self.panel_is_visible(panel_id) {
                self.toggle_panel_visibility(panel_id);
            }
        }
    }

    pub(super) fn context_menu_position(
        menu: DockTabContextMenu,
        window_width: f32,
        window_height: f32,
    ) -> (f32, f32) {
        // Keep the popup under the pointer when possible, but flip its position
        // toward the available space when it approaches a viewport edge.
        let viewport_width = window_width.max(1.0);
        let viewport_height = window_height.max(1.0);
        let popup_width = 184.0_f32.min(viewport_width);
        let popup_height = (if menu.header_only { 42.0_f32 } else { 132.0_f32 }).min(viewport_height);
        let left = menu.left.max(0.0).min((viewport_width - popup_width).max(0.0));
        let top = menu.top.max(0.0).min((viewport_height - popup_height).max(0.0));
        (left, top)
    }

    pub(super) fn dismiss_menus_if_outside(
        &mut self,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) -> bool {
        let menu_height = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        let (context_menu_left, context_menu_top) = self
            .tab_context_menu
            .map(|menu| Self::context_menu_position(menu, window_width, window_height))
            .unwrap_or((0.0, 0.0));
        let inside_context_menu = self.tab_context_menu.is_some_and(|menu| {
            let popup_height = (if menu.header_only { 42.0_f32 } else { 132.0_f32 }).min(window_height.max(1.0));
            pointer_x >= context_menu_left
                && pointer_x <= (context_menu_left + 184.0).min(window_width)
                && pointer_y >= context_menu_top
                && pointer_y <= (context_menu_top + popup_height).min(window_height)
        });
        let inside_windows_menu = self.windows_menu_open
            && pointer_x >= 0.0
            && pointer_x <= 224.0_f32.min(window_width)
            && pointer_y >= menu_height
            && pointer_y <= (menu_height + 136.0).min(window_height);
        // The Windows button itself is a trigger, not an outside click. It can
        // open or close that dropdown while still dismissing the other menu.
        let on_windows_button = pointer_x >= 0.0
            && pointer_x <= 112.0
            && pointer_y >= 0.0
            && pointer_y <= menu_height;
        if inside_context_menu || inside_windows_menu || on_windows_button {
            if on_windows_button && self.tab_context_menu.is_some() {
                self.tab_context_menu = None;
                return true;
            }
            return false;
        }

        let had_open_menu = self.tab_context_menu.is_some() || self.windows_menu_open;
        self.tab_context_menu = None;
        self.windows_menu_open = false;
        had_open_menu
    }

    pub(super) fn panel_is_visible(&self, panel_id: PanelId) -> bool {
        self.panel_visibility
            .iter()
            .find(|entry| entry.panel_id == panel_id)
            .map_or(true, |entry| entry.visible)
    }

    /// Discover current Core panel instances and ensure each has independent visibility state.
    ///
    /// Existing entries survive when their column temporarily disappears, so closing a
    /// column and restoring it can retain visibility preferences. A newly discovered
    /// panel instance reuses a stale entry only when the stable panel key and column match.
    pub(super) fn sync_panel_visibility_entries(&mut self) {
        let mut discovered: Vec<(usize, PanelId, String, String)> = Vec::new();

        for (column_index, column_id) in self.column_ids.iter().copied().enumerate() {
            for group_id in self.group_ids_in_column(column_id) {
                let Some(group) = self.tab_group_view_data(group_id) else {
                    continue;
                };
                for panel_id in group.panels {
                    let Some(instance) = self.layout.panel_instance(panel_id) else {
                        continue;
                    };
                    let column_label = self.column_label(column_index);
                    discovered.push((
                        column_index,
                        panel_id,
                        instance.key.as_str().to_owned(),
                        format!("{column_label} {}", self.panels.title(instance)),
                    ));
                }
            }
        }

        let current_panel_ids: Vec<PanelId> =
            discovered.iter().map(|(_, panel_id, _, _)| *panel_id).collect();

        for (column_index, panel_id, panel_key, title) in discovered {
            if let Some(entry) = self
                .panel_visibility
                .iter_mut()
                .find(|entry| entry.panel_id == panel_id)
            {
                entry.column_index = column_index;
                entry.panel_key = panel_key;
                entry.title = title;
                continue;
            }

            let stale_entry_index = self.panel_visibility.iter().position(|entry| {
                entry.column_index == column_index
                    && entry.panel_key == panel_key
                    && !current_panel_ids.contains(&entry.panel_id)
            });

            if let Some(index) = stale_entry_index {
                let entry = &mut self.panel_visibility[index];
                entry.panel_id = panel_id;
                entry.title = title;
            } else {
                self.panel_visibility.push(PanelVisibilityEntry {
                    panel_id,
                    column_index,
                    panel_key,
                    title,
                    visible: true,
                });
            }
        }
    }

    pub(super) fn toggle_panel_visibility(&mut self, panel_id: PanelId) {
        let Some(entry_index) = self
            .panel_visibility
            .iter()
            .position(|entry| entry.panel_id == panel_id)
        else {
            return;
        };
        let column_index = self.panel_visibility[entry_index].column_index;
        let panel_key = self.panel_visibility[entry_index].panel_key.clone();
        let next_visible = !self.panel_visibility[entry_index].visible;
        self.panel_visibility[entry_index].visible = next_visible;

        if next_visible && !self.column_open[column_index] {
            // Restore the standard dock template, then bind its freshly-created Core
            // panel IDs back to the existing visibility entries by their stable keys.
            if let Err(error) = self.open_column_from_menu(column_index, &panel_key) {
                eprintln!("Could not reopen dock column: {error}");
                if let Some(entry) = self.panel_visibility.get_mut(entry_index) {
                    entry.visible = false;
                }
            }
            return;
        }
        if !self.column_open[column_index] {
            return;
        }

        let column_has_visible_panels = self
            .panel_visibility
            .iter()
            .any(|entry| entry.column_index == column_index && entry.visible);
        if !column_has_visible_panels {
            // Closing the last visible panel closes every group in this column.
            // Core normalization then removes the empty Column and collapses its Split.
            self.close_column(column_index);
            return;
        }

        if next_visible {
            return;
        }

        let group = self
            .group_ids_in_column(self.column_ids[column_index])
            .into_iter()
            .find(|group_id| {
                self.tab_group_view_data(*group_id)
                    .is_some_and(|data| data.panels.contains(&panel_id))
            });
        let Some(group) = group else {
            return;
        };
        if self
            .active_panel_id(group)
            .is_some_and(|active| self.panel_is_visible(active))
        {
            return;
        }
        if let Some(next_panel) = self
            .tab_group_view_data(group)
            .into_iter()
            .flat_map(|data| data.panels)
            .find(|candidate| self.panel_is_visible(*candidate))
        {
            self.activate_tab(group, next_panel);
        }
    }

    fn collapse_column_to_icons(&mut self, column_index: usize) {
        if column_index >= self.column_ids.len()
            || matches!(
                self.layout.node(self.column_ids[column_index]),
                Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
            )
        {
            return;
        }
        if let Err(error) = self.apply_dock_command(
            DockCommand::SetColumnMode {
                column: self.column_ids[column_index],
                mode: ColumnMode::IconsOnly,
            }) {
            eprintln!("Could not collapse dock column to icons: {error}");
        }
    }

    pub(super) fn collapse_context_to_icons(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        self.collapse_column_to_icons(menu.column_index);
    }

    pub(super) fn expand_context_tabs(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        if menu.column_index >= self.column_ids.len() {
            return;
        }
        if let Err(error) = self.apply_dock_command(
            DockCommand::SetColumnMode {
                column: self.column_ids[menu.column_index],
                mode: ColumnMode::Expanded,
            }) {
            eprintln!("Could not expand dock tabs: {error}");
        }
    }

    pub(super) fn toggle_context_group_collapse(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let collapsed = !self.group_is_collapsed(menu.group);
        if let Err(error) = self.apply_dock_command(
            DockCommand::CollapseGroup {
                group: menu.group,
                collapsed,
            }) {
            eprintln!("Could not change dock group collapse state: {error}");
        }
    }
}
