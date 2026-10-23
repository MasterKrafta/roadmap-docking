use dock_core::{DockCommand, DockSide, FloatingGroupId, Node, NodeId, Point, Rect, Size};

use super::MainWindow;

impl MainWindow {
    pub(super) fn composition_column_indices(&self, root: NodeId) -> Vec<usize> {
        self.column_indices_in_core_order(root)
    }

    fn composition_width(&self, root: NodeId) -> Option<f32> {
        let columns = self.composition_column_indices(root);
        if columns.is_empty() {
            return None;
        }
        let splitter_gaps = columns.len().saturating_sub(1) as f32;
        let width = columns
            .iter()
            .map(|column_index| self.column_width(*column_index))
            .sum::<f32>()
            + self.theme.metrics.splitter_thickness * splitter_gaps;
        Some(width.max(1.0))
    }

    fn composition_size(&self, root: NodeId, window_height: f32) -> Option<Size> {
        let width = self.composition_width(root)?;
        let menu_height = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        let available_height = (window_height - menu_height).max(1.0);
        // Floating docks should be compact enough to move vertically instead
        // of initially filling almost the entire application viewport.
        let floating_height = available_height.min((window_height * 0.75).max(1.0));
        Some(Size::new(width, floating_height.max(1.0)))
    }

    pub(super) fn sync_floating_composition_bounds(&mut self) {
        let Some(group_id) = self.floating_composition else {
            return;
        };
        let Some((root, current_bounds)) = self
            .layout
            .floating_group(group_id)
            .and_then(|floating| {
                self.layout
                    .dock_area(floating.area)
                    .map(|area| (area.root, floating.bounds))
            })
        else {
            self.floating_composition = None;
            return;
        };
        let Some(width) = self.composition_width(root) else {
            return;
        };
        if (current_bounds.size.width - width).abs() <= f32::EPSILON {
            return;
        }
        let bounds = Rect::new(
            current_bounds.origin,
            Size::new(width, current_bounds.size.height),
        );
        if let Err(error) = self.layout.apply(
            DockCommand::ResizeFloatingBounds { group: group_id, bounds },
            &self.dock_metrics,
        ) {
            eprintln!("Could not synchronize floating dock composition bounds: {error}");
        }
    }

    pub(super) fn begin_composition_drag(
        &mut self,
        column_index: usize,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) {
        self.composition_drag_pending = false;
        self.composition_dragging = false;
        self.composition_drag_source = None;
        self.composition_drag_group = None;
        self.composition_drag_start_x = pointer_x;
        self.composition_drag_start_y = pointer_y;

        if let Some(group_id) = self.floating_composition {
            let floating_root = self
                .layout
                .floating_group(group_id)
                .and_then(|group| self.layout.dock_area(group.area))
                .map(|area| area.root);
            let clicked_column_is_floating = floating_root.is_some_and(|root| {
                self.composition_column_indices(root).contains(&column_index)
            });

            if clicked_column_is_floating {
                if let Some(origin) = self
                    .layout
                    .floating_group(group_id)
                    .map(|group| group.bounds.origin)
                {
                    self.composition_drag_group = Some(group_id);
                    self.composition_drag_offset_x = pointer_x - origin.x;
                    self.composition_drag_offset_y = pointer_y - origin.y;
                    self.composition_drag_pending = true;
                    return;
                }
                self.floating_composition = None;
            } else if floating_root.is_some() {
                // This UI currently tracks one movable floating composition.
                // Do not let a main-dock header accidentally drag that floating window.
                return;
            } else {
                self.floating_composition = None;
            }
        }

        let Some(root) = self.layout.main_area(DockSide::Right).map(|area| area.root) else {
            return;
        };
        let Some(size) = self.composition_size(root, window_height) else {
            return;
        };
        let origin_x = (window_width - size.width).max(0.0);
        let origin_y = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        self.composition_drag_offset_x = pointer_x - origin_x;
        self.composition_drag_offset_y = pointer_y - origin_y;
        self.composition_drag_source = Some(root);
        self.composition_drag_pending = true;
    }

    pub(super) fn begin_tab_group_drag(
        &mut self,
        column_index: usize,
        group_id: NodeId,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        if let Some(floating_id) = self.floating_composition {
            let floating_root = self
                .layout
                .floating_group(floating_id)
                .and_then(|floating| self.layout.dock_area(floating.area))
                .map(|area| area.root);
            if floating_root == Some(group_id) {
                let Some(origin) = self
                    .layout
                    .floating_group(floating_id)
                    .map(|floating| floating.bounds.origin)
                else {
                    return;
                };
                self.composition_drag_pending = false;
                self.composition_dragging = false;
                self.composition_drag_source = None;
                self.composition_drag_group = Some(floating_id);
                self.composition_drag_start_x = pointer_x;
                self.composition_drag_start_y = pointer_y;
                self.composition_drag_offset_x = pointer_x - origin.x;
                self.composition_drag_offset_y = pointer_y - origin.y;
                self.composition_drag_pending = true;
            }
            return;
        }
        if !self.main_column_indices_in_core_order().contains(&column_index) {
            return;
        }
        let Some(column_id) = self.column_ids.get(column_index).copied() else {
            return;
        };
        if !self.group_ids_in_column(column_id).contains(&group_id) {
            return;
        }

        self.composition_drag_pending = false;
        self.composition_dragging = false;
        self.composition_drag_source = Some(group_id);
        self.composition_drag_group = None;
        self.composition_drag_start_x = pointer_x;
        self.composition_drag_start_y = pointer_y;
        self.composition_drag_offset_x = self.column_width(column_index) / 2.0;
        self.composition_drag_offset_y = self.theme.metrics.tab_strip_height / 2.0;
        self.composition_drag_pending = true;
    }

    fn floating_origin_for_pointer(
        &self,
        pointer_x: f32,
        pointer_y: f32,
        size: Size,
        window_width: f32,
        window_height: f32,
    ) -> Point {
        // Keep the floating composition horizontally reachable and its
        // bottom edge inside the application viewport.
        let visible_width = 80.0_f32.min(size.width.max(1.0));
        let min_x = -(size.width - visible_width).max(0.0);
        let max_x = (window_width - visible_width).max(0.0);
        let min_y = 0.0_f32;
        let max_y = (window_height - size.height).max(0.0);
        Point::new(
            (pointer_x - self.composition_drag_offset_x).clamp(min_x, max_x),
            (pointer_y - self.composition_drag_offset_y).clamp(min_y, max_y),
        )
    }

    pub(super) fn float_main_composition_at_pointer(
        &mut self,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) -> Option<FloatingGroupId> {
        let node = self.composition_drag_source?;
        let size = match self.layout.node(node) {
            Some(Node::Tabs(_)) => {
                let width = self
                    .column_ids
                    .iter()
                    .enumerate()
                    .find_map(|(column_index, column_id)| {
                        self.group_ids_in_column(*column_id)
                            .contains(&node)
                            .then(|| self.column_width(column_index))
                    })
                    .unwrap_or(self.theme.metrics.default_column_width);
                let menu_height = self
                    .theme
                    .metrics
                    .column_header_height
                    .min(window_height.max(1.0));
                let available_height = (window_height - menu_height).max(1.0);
                Some(Size::new(
                    width.max(1.0),
                    available_height.min((window_height * 0.75).max(1.0)).max(1.0),
                ))
            }
            _ => self.composition_size(node, window_height),
        }?;
        let origin = self.floating_origin_for_pointer(
            pointer_x,
            pointer_y,
            size,
            window_width,
            window_height,
        );
        let bounds = Rect::new(origin, size);
        if let Err(error) = self.apply_dock_command(
            DockCommand::Float { node, bounds }) {
            eprintln!("Could not float dock composition: {error}");
            return None;
        }

        let floating_id = self.layout.floating_groups().find_map(|(group_id, group)| {
            self.layout
                .dock_area(group.area)
                .is_some_and(|area| area.root == node)
                .then_some(group_id)
        })?;
        self.floating_composition = Some(floating_id);
        self.composition_drag_group = Some(floating_id);
        self.composition_drag_source = None;
        Some(floating_id)
    }

    pub(super) fn move_floating_composition(
        &mut self,
        group_id: FloatingGroupId,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) {
        let Some(size) = self
            .layout
            .floating_group(group_id)
            .map(|group| group.bounds.size)
        else {
            self.floating_composition = None;
            self.composition_drag_group = None;
            self.composition_dragging = false;
            return;
        };
        let origin = self.floating_origin_for_pointer(
            pointer_x,
            pointer_y,
            size,
            window_width,
            window_height,
        );
        if let Err(error) = self.apply_dock_command(
            DockCommand::MoveFloating {
                group: group_id,
                origin,
            }) {
            eprintln!("Could not move floating dock composition: {error}");
        }
    }
}
