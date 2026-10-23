use dock_core::{ColumnMode, DockCommand, DockSide, FloatingGroupId, Node, NodeId, PanelId, Point, Rect, Size};

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
        let floating_group_ids: Vec<_> =
            self.layout.floating_groups().map(|(group_id, _)| group_id).collect();
        for group_id in floating_group_ids {
            let Some((root, current_bounds)) = self
                .layout
                .floating_group(group_id)
                .and_then(|floating| {
                    self.layout
                        .dock_area(floating.area)
                        .map(|area| (area.root, floating.bounds))
                })
            else {
                continue;
            };
            let Some(width) = self.composition_width(root) else {
                // A standalone Tabs root keeps its explicitly assigned width.
                continue;
            };
            if (current_bounds.size.width - width).abs() <= f32::EPSILON {
                continue;
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
    }

    fn node_tree_contains(&self, root: NodeId, target: NodeId) -> bool {
        if root == target {
            return true;
        }
        match self.layout.node(root) {
            Some(Node::Split(split)) => split
                .children
                .iter()
                .any(|child| self.node_tree_contains(*child, target)),
            Some(Node::Column(column)) => column
                .groups
                .iter()
                .any(|group| self.node_tree_contains(*group, target)),
            Some(Node::Tabs(tabs)) => tabs
                .children
                .iter()
                .any(|child| self.node_tree_contains(*child, target)),
            Some(Node::Panel(_)) | None => false,
        }
    }

    fn floating_group_containing_node(
        &self,
        target: NodeId,
    ) -> Option<(FloatingGroupId, NodeId, Rect)> {
        self.layout.floating_groups().find_map(|(group_id, floating)| {
            let root = self.layout.dock_area(floating.area)?.root;
            self.node_tree_contains(root, target)
                .then_some((group_id, root, floating.bounds))
        })
    }

    fn column_index_for_group(&self, group_id: NodeId) -> Option<usize> {
        self.column_ids.iter().position(|column_id| {
            self.group_ids_in_column(*column_id).contains(&group_id)
        })
    }

    fn column_index_for_panel(&self, panel_id: PanelId) -> Option<usize> {
        self.column_ids.iter().enumerate().find_map(|(column_index, column_id)| {
            self.group_ids_in_column(*column_id).into_iter().any(|group_id| {
                self.tab_group_view_data(group_id)
                    .is_some_and(|group| group.panels.contains(&panel_id))
            }).then_some(column_index)
        })
    }

    fn panel_node_id(&self, panel_id: PanelId) -> Option<NodeId> {
        self.layout.nodes().find_map(|(node_id, node)| match node {
            Node::Panel(panel) if panel.panel_id == panel_id => Some(node_id),
            _ => None,
        })
    }

    fn detached_column_width(&self, column_index: Option<usize>) -> f32 {
        let default_width = self.theme.metrics.default_column_width;
        let Some(column_id) = column_index.and_then(|index| self.column_ids.get(index).copied()) else {
            return default_width;
        };
        let Some(Node::Column(column)) = self.layout.node(column_id) else {
            return default_width;
        };
        let requested_width = if column.mode == ColumnMode::Expanded {
            column.width
        } else {
            column.last_expanded_width.unwrap_or(default_width)
        };
        if requested_width.is_finite() && requested_width > 0.0 {
            requested_width.max(self.dock_metrics.min_column_width)
        } else {
            default_width
        }
    }

    fn single_column_size(&self, width: f32, window_height: f32) -> Size {
        let header_height = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        let available_height = (window_height - header_height).max(1.0);
        let floating_height = available_height.min((window_height * 0.75).max(1.0));
        Size::new(width.max(1.0), floating_height.max(1.0))
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

        let Some(column_id) = self.column_ids.get(column_index).copied() else {
            return;
        };
        if let Some((group_id, _root, bounds)) = self.floating_group_containing_node(column_id) {
            self.composition_drag_group = Some(group_id);
            self.composition_drag_offset_x = pointer_x - bounds.origin.x;
            self.composition_drag_offset_y = pointer_y - bounds.origin.y;
            self.composition_drag_pending = true;
            return;
        }

        let Some(root) = self.layout.main_area(DockSide::Right).map(|area| area.root) else {
            return;
        };
        if !self.node_tree_contains(root, column_id) {
            return;
        }
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
        self.composition_drag_source = Some(CompositionDragSource::Composition(root));
        self.composition_drag_pending = true;
    }

    pub(super) fn begin_tab_group_drag(
        &mut self,
        column_index: usize,
        group_id: NodeId,
        pointer_x: f32,
        pointer_y: f32,
        column_origin_x: f32,
        group_origin_y: f32,
        composition_origin: Point,
    ) {
        self.composition_drag_pending = false;
        self.composition_dragging = false;
        self.composition_drag_source = None;
        self.composition_drag_group = None;
        self.composition_drag_start_x = pointer_x;
        self.composition_drag_start_y = pointer_y;

        if let Some((floating_id, floating_root, bounds)) =
            self.floating_group_containing_node(group_id)
        {
            // Legacy floating Tabs roots and single-group floating Columns already
            // represent a complete composition: move the window instead of trying
            // to detach its only group.
            let source_column_index = self.column_index_for_group(group_id);
            let source_group_count = source_column_index
                .map(|index| self.group_ids_in_column(self.column_ids[index]).len());
            if floating_root == group_id || source_group_count.is_none_or(|count| count <= 1) {
                self.composition_drag_group = Some(floating_id);
                self.composition_drag_offset_x = pointer_x - bounds.origin.x;
                self.composition_drag_offset_y = pointer_y - bounds.origin.y;
                self.composition_drag_pending = true;
                return;
            }

            // A group inside an already-floating multi-group Column follows the
            // same detach rule as a group in the main dock. Only the selected
            // group becomes a new floating Column; its siblings stay behind.
            self.composition_drag_source = Some(CompositionDragSource::Group(group_id));
            self.composition_drag_offset_x = pointer_x - column_origin_x;
            self.composition_drag_offset_y = pointer_y - group_origin_y
                + self.theme.metrics.column_header_height;
            self.composition_drag_pending = true;
            return;
        }

        let Some(column_id) = self.column_ids.get(column_index).copied() else {
            return;
        };
        if !self.group_ids_in_column(column_id).contains(&group_id) {
            return;
        }

        let group_count = self.group_ids_in_column(column_id).len();
        if let Some(main_root) = self.layout.main_area(DockSide::Right).map(|area| area.root) {
            if self.node_tree_contains(main_root, group_id) {
                if group_count <= 1 {
                    // The last group cannot leave behind an empty Column. Detach
                    // the containing composition as a whole.
                    self.composition_drag_source =
                        Some(CompositionDragSource::Composition(main_root));
                    self.composition_drag_offset_x = pointer_x - composition_origin.x;
                    self.composition_drag_offset_y = pointer_y - composition_origin.y;
                } else {
                    self.composition_drag_source = Some(CompositionDragSource::Group(group_id));
                    self.composition_drag_offset_x = pointer_x - column_origin_x;
                    self.composition_drag_offset_y = pointer_y - group_origin_y
                        + self.theme.metrics.column_header_height;
                }
                self.composition_drag_pending = true;
            }
        } else if group_count > 1 {
            self.composition_drag_source = Some(CompositionDragSource::Group(group_id));
            self.composition_drag_offset_x = pointer_x - column_origin_x;
            self.composition_drag_offset_y = pointer_y - group_origin_y
                + self.theme.metrics.column_header_height;
            self.composition_drag_pending = true;
        }
    }

    pub(super) fn begin_panel_drag(
        &mut self,
        column_index: usize,
        group_id: NodeId,
        panel_id: PanelId,
        pointer_x: f32,
        pointer_y: f32,
        tab_origin_x: f32,
        tab_origin_y: f32,
        column_origin_x: f32,
        group_origin_y: f32,
        composition_origin: Point,
        content_padding: f32,
    ) {
        let Some(group) = self.tab_group_view_data(group_id) else {
            return;
        };
        if !group.panels.contains(&panel_id) {
            return;
        }
        if group.panels.len() <= 1 {
            self.begin_tab_group_drag(
                column_index,
                group_id,
                pointer_x,
                pointer_y,
                column_origin_x,
                group_origin_y,
                composition_origin,
            );
            return;
        }

        self.composition_drag_pending = false;
        self.composition_dragging = false;
        self.composition_drag_group = None;
        self.composition_drag_source = Some(CompositionDragSource::Panel(panel_id));
        self.composition_drag_start_x = pointer_x;
        self.composition_drag_start_y = pointer_y;
        // The detached tab becomes the first tab in a new group. Preserve the
        // grab point inside that tab while accounting for its new Column header.
        self.composition_drag_offset_x = pointer_x - tab_origin_x + content_padding;
        self.composition_drag_offset_y =
            pointer_y - tab_origin_y + self.theme.metrics.column_header_height;
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
        let source = self.composition_drag_source?;
        let target_node = match source {
            CompositionDragSource::Composition(node) | CompositionDragSource::Group(node) => node,
            CompositionDragSource::Panel(panel_id) => self.panel_node_id(panel_id)?,
        };
        let size = match source {
            CompositionDragSource::Composition(node) => self.composition_size(node, window_height)?,
            CompositionDragSource::Group(group_id) => {
                let width = self.detached_column_width(self.column_index_for_group(group_id));
                self.single_column_size(width, window_height)
            }
            CompositionDragSource::Panel(panel_id) => {
                let width = self.detached_column_width(self.column_index_for_panel(panel_id));
                self.single_column_size(width, window_height)
            }
        };
        let origin = self.floating_origin_for_pointer(
            pointer_x,
            pointer_y,
            size,
            window_width,
            window_height,
        );
        let bounds = Rect::new(origin, size);
        let command = match source {
            CompositionDragSource::Composition(node) => DockCommand::Float { node, bounds },
            CompositionDragSource::Group(group) => DockCommand::FloatGroupAsColumn { group, bounds },
            CompositionDragSource::Panel(panel) => DockCommand::FloatPanelAsGroup { panel, bounds },
        };
        if let Err(error) = self.apply_dock_command(command) {
            eprintln!("Could not float dock composition: {error}");
            return None;
        }

        let floating_id = self.layout.floating_groups().find_map(|(group_id, floating)| {
            let root = self.layout.dock_area(floating.area)?.root;
            self.node_tree_contains(root, target_node).then_some(group_id)
        })?;
        self.composition_drag_group = Some(floating_id);
        self.composition_drag_source = None;
        self.sync_column_slots_from_core();
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
