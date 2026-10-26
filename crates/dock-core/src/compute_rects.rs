use std::collections::HashMap;

use crate::{
    Axis, Column, ColumnMode, ColumnRect, ComputeRectsError, DockMetrics, DockSide,
    FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, GeometryError, GroupRect, Layout,
    LayoutRects, Node, NodeId, PanelRect, Point, Rect, ResizeEdge, Size, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, Tabs, VerticalOverflow,
};

struct StackAllocation {
    sizes: Vec<f32>,
    content_height: f32,
    overflows: bool,
}

#[cfg(test)]
mod visit_probe {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Mutex, OnceLock,
        },
    };

    use crate::NodeId;

    #[derive(Clone, Copy, Default)]
    pub(super) struct NodeVisitCounts {
        pub(super) preferred_width: u64,
        pub(super) minimum_node_size_helper: u64,
        pub(super) recursive_minimum_node_size: u64,
        pub(super) allocate_stack_child: u64,
    }

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static COUNTS: OnceLock<Mutex<HashMap<NodeId, NodeVisitCounts>>> = OnceLock::new();

    fn counts() -> &'static Mutex<HashMap<NodeId, NodeVisitCounts>> {
        COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn record(node: NodeId, update: impl FnOnce(&mut NodeVisitCounts)) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut counts) = counts().lock() {
            update(counts.entry(node).or_default());
        }
    }

    pub(super) fn record_preferred_width(node: NodeId) {
        record(node, |counts| counts.preferred_width += 1);
    }

    pub(super) fn record_minimum_node_size_helper(node: NodeId) {
        record(node, |counts| counts.minimum_node_size_helper += 1);
    }

    pub(super) fn record_recursive_minimum_node_size(node: NodeId) {
        record(node, |counts| counts.recursive_minimum_node_size += 1);
    }

    pub(super) fn record_allocate_stack_child(node: NodeId) {
        record(node, |counts| counts.allocate_stack_child += 1);
    }

    pub(super) fn start() {
        ENABLED.store(false, Ordering::Relaxed);
        counts()
            .lock()
            .expect("visit-counter mutex must not be poisoned")
            .clear();
        ENABLED.store(true, Ordering::Relaxed);
    }

    pub(super) fn finish() -> HashMap<NodeId, NodeVisitCounts> {
        ENABLED.store(false, Ordering::Relaxed);
        counts()
            .lock()
            .expect("visit-counter mutex must not be poisoned")
            .clone()
    }
}
#[cfg(test)]
pub(crate) fn record_recursive_minimum_node_size(node: NodeId) {
    visit_probe::record_recursive_minimum_node_size(node);
}

impl Layout {
    /// Resolves the validated model into a snapshot of logical-pixel rectangles.
    ///
    /// Column widths come from the model for Expanded columns and from the
    /// presentation metrics for the two rail modes.
    /// The Canvas receives the remaining horizontal space. If that remainder is
    /// below min_canvas_width, computation fails instead of silently compressing
    /// columns or the Canvas. Vertical content that cannot fit is reported with
    /// a scroll extent. Floating bounds are clamped in this snapshot only; the
    /// persistent Layout and CanvasState are not mutated.
    pub fn compute_rects(
        &self,
        viewport: Rect,
        metrics: &DockMetrics,
    ) -> Result<LayoutRects, ComputeRectsError> {
        metrics
            .validate()
            .map_err(|reason| ComputeRectsError::InvalidMetrics { reason })?;
        viewport
            .validate_non_empty()
            .map_err(ComputeRectsError::Geometry)?;
        self.validate_cached(metrics)
            .map_err(|errors| ComputeRectsError::InvalidLayout { errors })?;

        // Cache minimum sizes only for this immutable geometry pass. The cache
        // cannot outlive the exact layout and metrics used to calculate it.
        let mut minimum_sizes = HashMap::new();

        // Hidden chrome changes presentation only. Keep the main-dock tree in
        // Layout, but omit its geometry so the Canvas gets the full viewport.
        let (left_root, right_root) = if self.chrome().dock_visible() {
            (
                self.main_area(DockSide::Left).map(|area| area.root),
                self.main_area(DockSide::Right).map(|area| area.root),
            )
        } else {
            (None, None)
        };
        let left_width = match left_root {
            Some(root) => preferred_width(self, root, metrics, &mut minimum_sizes)?,
            None => 0.0,
        };
        let right_width = match right_root {
            Some(root) => preferred_width(self, root, metrics, &mut minimum_sizes)?,
            None => 0.0,
        };

        let available_canvas = viewport.size.width - left_width - right_width;
        if !available_canvas.is_finite() {
            return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
        }
        if available_canvas < metrics.min_canvas_width {
            return Err(ComputeRectsError::InsufficientCanvasWidth {
                available: available_canvas,
                minimum: metrics.min_canvas_width,
            });
        }

        let viewport_right = viewport
            .checked_right()
            .map_err(ComputeRectsError::Geometry)?;
        let canvas_x = viewport.origin.x + left_width;
        let right_x = viewport_right - right_width;
        let canvas = Rect::new(
            Point::new(canvas_x, viewport.origin.y),
            Size::new(available_canvas, viewport.size.height),
        );
        canvas.validate().map_err(ComputeRectsError::Geometry)?;

        let mut output = LayoutRects {
            viewport: Some(viewport),
            canvas: Some(canvas),
            ..LayoutRects::default()
        };

        if let Some(root) = left_root {
            let area = Rect::new(viewport.origin, Size::new(left_width, viewport.size.height));
            layout_node(self, root, area, metrics, &mut minimum_sizes, &mut output)?;
        }
        if let Some(root) = right_root {
            let area = Rect::new(
                Point::new(right_x, viewport.origin.y),
                Size::new(right_width, viewport.size.height),
            );
            layout_node(self, root, area, metrics, &mut minimum_sizes, &mut output)?;
        }

        // A rail flyout belongs to the dock presentation. Keep its transient
        // state while hidden, but do not emit geometry until the dock is shown.
        if self.chrome().dock_visible()
            && let Some(flyout) = self.flyout()
        {
            output.flyout = Some(compute_flyout_rect(self, flyout, viewport, &output)?);
        }

        // Floating windows are part of the dock chrome presentation too.
        // Their model bounds and roots stay in Layout while hidden, but neither
        // window geometry nor child-panel geometry is emitted in hidden modes.
        if self.chrome().dock_visible() {
            // Preserve deterministic back-to-front order in the geometry snapshot.
            // A single floating window may not fit the current client area. Build
            // its geometry separately so a local failure cannot discard the main
            // dock or leave partially emitted rectangles for that window.
            for (id, floating) in self.floating_groups_in_paint_order() {
                let area = self
                    .dock_area(floating.area)
                    .ok_or(ComputeRectsError::InvalidLayoutState)?;
                let mut floating_output = LayoutRects::default();
                let result = (|| {
                    let bounds = clamp_floating_bounds_with_cache(
                        self,
                        area.root,
                        floating.bounds,
                        viewport,
                        metrics,
                        &mut minimum_sizes,
                    )?;
                    floating_output
                        .floating_groups
                        .push(FloatingGroupRect::new(id, bounds, floating.z_order));
                    push_resize_edges(id, bounds, metrics, &mut floating_output)?;
                    layout_floating_root(
                        self,
                        area.root,
                        bounds,
                        metrics,
                        &mut minimum_sizes,
                        &mut floating_output,
                    )?;
                    Ok::<(), ComputeRectsError>(())
                })();

                match result {
                    Ok(()) => append_floating_geometry(&mut output, floating_output),
                    Err(error) => {
                        tracing::warn!(
                            floating_group = ?id,
                            error = %error,
                            "omitting floating group from geometry snapshot"
                        );
                    }
                }
            }
        }

        Ok(output)
    }
}

fn compute_flyout_rect(
    layout: &Layout,
    flyout: crate::FlyoutState,
    viewport: Rect,
    output: &LayoutRects,
) -> Result<FlyoutRect, ComputeRectsError> {
    let rail = output
        .columns
        .iter()
        .find(|column| column.column == flyout.column)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;
    let canvas = output.canvas.ok_or(ComputeRectsError::InvalidLayoutState)?;
    let canvas_right = canvas
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let rail_right = rail
        .rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;

    // A flyout expands from its rail toward the Canvas and overlays it; it
    // never changes the width allocated to the Canvas itself.
    let (resize_edge, anchor_x, available_width) = if rail_right <= canvas.origin.x {
        (ResizeEdge::Right, rail_right, canvas.size.width)
    } else if rail.rect.origin.x >= canvas_right {
        (ResizeEdge::Left, rail.rect.origin.x, canvas.size.width)
    } else {
        // Flyouts are anchored to main-dock rails, not floating columns.
        return Err(ComputeRectsError::InvalidLayoutState);
    };

    let instance = layout
        .panel_instance(flyout.panel)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;
    let constraints = instance.constraints;
    let min_width = constraints.min_size.width;
    let preferred_width = layout
        .flyout_width(&instance.key)
        .or_else(|| instance.state.last_known_size().map(|size| size.width))
        .unwrap_or(constraints.preferred_size.width);
    let max_width = constraints
        .max_size
        .map(|size| size.width)
        .unwrap_or(f32::MAX)
        .min(available_width);
    if !available_width.is_finite() || max_width < min_width {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: available_width,
            required: min_width,
        });
    }

    let width = preferred_width.clamp(min_width, max_width);
    let x = match resize_edge {
        ResizeEdge::Right => anchor_x,
        ResizeEdge::Left => anchor_x - width,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let rect = Rect::new(
        Point::new(x, viewport.origin.y),
        Size::new(width, viewport.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;

    Ok(FlyoutRect::new(
        flyout.column,
        flyout.group,
        flyout.panel,
        rect,
        resize_edge,
    ))
}

fn preferred_width(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<f32, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_preferred_width(id);
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayoutState)?;
    let width = match node {
        Node::Column(column) => effective_column_width(column, metrics),
        Node::Tabs(_) | Node::Panel(_) => metrics
            .default_column_width
            .max(minimum_node_size(layout, id, metrics, minimum_sizes)?.width),
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                width += preferred_width(layout, *child, metrics, minimum_sizes)?;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            width
        }
        Node::Split(split) => {
            let mut width = 0.0_f32;
            for child in &split.children {
                width = width.max(preferred_width(layout, *child, metrics, minimum_sizes)?);
            }
            width
        }
    };
    if width.is_finite() && width > 0.0 {
        Ok(width)
    } else {
        Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ))
    }
}

fn effective_column_width(column: &Column, metrics: &DockMetrics) -> f32 {
    match column.mode {
        ColumnMode::Expanded => column.width,
        ColumnMode::IconsAndLabels => metrics.min_column_width,
        ColumnMode::IconsOnly => metrics.rail_width,
    }
}

/// Computes the minimum geometry of a node, including panel-content constraints
/// and the chrome consumed by expanded Tabs groups.
fn minimum_node_size(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Size, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_minimum_node_size_helper(id);
    layout
        .minimum_node_size_with_cache(id, metrics, minimum_sizes)
        .map_err(|error| match error {
            crate::CommandError::InvalidValue {
                field: "size",
                reason: "computed minimum size overflowed",
            } => ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow),
            crate::CommandError::InvalidValue { field: "size", .. } => {
                ComputeRectsError::InvalidLayoutState
            }
            _ => ComputeRectsError::InvalidLayoutState,
        })
}

fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayoutState)?;

    // Columns and vertical splits manage their own stack overflow. Other nodes
    // need an expanded content rect when the parent viewport is shorter than
    // their minimum geometry, with a separate viewport/content extent record.
    let manages_vertical_stack = matches!(node, Node::Column(_))
        || matches!(node, Node::Split(split) if split.axis == Axis::Y);
    let mut content_rect = rect;
    if !manages_vertical_stack {
        let minimum = minimum_node_size(layout, id, metrics, minimum_sizes)?;
        if content_rect.size.height < minimum.height {
            push_vertical_overflow(id, rect, minimum.height, output)?;
            content_rect.size.height = minimum.height;
            content_rect
                .validate()
                .map_err(ComputeRectsError::Geometry)?;
        }
    }

    match node {
        Node::Column(column) => layout_column(
            layout,
            id,
            column,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Tabs(tabs) => layout_tabs(
            layout,
            id,
            tabs,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Panel(panel) => {
            let minimum = layout
                .panel_instance(panel.panel_id)
                .ok_or(ComputeRectsError::InvalidLayoutState)?
                .constraints
                .min_size;
            if content_rect.size.width < minimum.width {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::X,
                    available: content_rect.size.width,
                    required: minimum.width,
                });
            }
            if content_rect.size.height < minimum.height {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::Y,
                    available: content_rect.size.height,
                    required: minimum.height,
                });
            }
            output
                .panels
                .push(PanelRect::new(panel.panel_id, content_rect));
            Ok(())
        }
        Node::Split(split) => layout_split(
            layout,
            id,
            split,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
    }
}

fn layout_column(
    layout: &Layout,
    id: NodeId,
    column: &Column,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    output.columns.push(ColumnRect::new(id, rect));
    if column.groups.is_empty() {
        return Ok(());
    }

    // Rail modes flatten tabs into fixed-height panel entries. Their order is
    // canonical: groups top-to-bottom, then tabs inside each group left-to-right.
    // Only geometry changes; the underlying Tabs/Column tree remains untouched.
    if column.mode.is_rail() {
        let slot_height = metrics.effective_collapsed_group_height();
        let display_order = layout
            .column_display_order(id)
            .map_err(|_| ComputeRectsError::InvalidLayoutState)?;
        let required = slot_height * display_order.len() as f32;
        if !required.is_finite() {
            return Err(ComputeRectsError::Geometry(
                GeometryError::ArithmeticOverflow,
            ));
        }
        if rect.size.height < required {
            push_vertical_overflow(id, rect, required, output)?;
        }

        let mut y = rect.origin.y;
        let mut item_index = 0usize;
        while item_index < display_order.len() {
            let group_id = display_order[item_index].group;
            let group_start = y;
            let mut item_count = 0usize;

            while item_index < display_order.len() && display_order[item_index].group == group_id {
                let item = display_order[item_index];
                let item_rect = Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, slot_height),
                );
                item_rect.validate().map_err(ComputeRectsError::Geometry)?;
                output.tab_headers.push(TabHeaderRect::new(
                    item.group,
                    item.panel,
                    item.tab_index,
                    item_rect,
                ));
                y += slot_height;
                item_count += 1;
                item_index += 1;
            }

            let group_rect = Rect::new(
                Point::new(rect.origin.x, group_start),
                Size::new(rect.size.width, slot_height * item_count as f32),
            );
            group_rect.validate().map_err(ComputeRectsError::Geometry)?;
            output.groups.push(GroupRect::new(group_id, group_rect));
        }
        return Ok(());
    }

    let allocation = allocate_stack_heights(
        layout,
        &column.groups,
        &column.group_ratios,
        rect.size.height,
        Axis::Y,
        metrics,
        minimum_sizes,
    )?;
    if allocation.overflows {
        push_vertical_overflow(id, rect, allocation.content_height, output)?;
    }

    let mut y = rect.origin.y;
    for (index, group_id) in column.groups.iter().enumerate() {
        let height = allocation.sizes[index];
        let group_rect = Rect::new(
            Point::new(rect.origin.x, y),
            Size::new(rect.size.width, height),
        );
        layout_node(layout, *group_id, group_rect, metrics, minimum_sizes, output)?;
        y += height;

        if index + 1 < column.groups.len() {
            push_splitter(id, index, Axis::Y, y, rect, metrics, output)?;
            y += metrics.splitter_thickness;
        }
    }
    Ok(())
}

fn layout_tabs(
    layout: &Layout,
    id: NodeId,
    tabs: &Tabs,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let collapsed = tabs.collapsed;
    let minimum_size = minimum_node_size(layout, id, metrics, minimum_sizes)?;
    // Geometry and resize commands share Layout::minimum_node_size, including
    // the effective tab-strip floor for collapsed groups.
    let minimum_height = minimum_size.height;
    if rect.size.width < minimum_size.width && !collapsed {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: rect.size.width,
            required: minimum_size.width,
        });
    }
    if rect.size.height < minimum_height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::Y,
            available: rect.size.height,
            required: minimum_height,
        });
    }

    // A collapsed Tabs group remains strip-sized even when its parent container
    // (for example a floating window) is taller than the collapsed group.
    let group_rect = if collapsed {
        Rect::new(rect.origin, Size::new(rect.size.width, minimum_size.height))
    } else {
        rect
    };
    output.groups.push(GroupRect::new(id, group_rect));

    let strip_height = if collapsed {
        group_rect.size.height
    } else {
        metrics.tab_strip_height
    };
    let strip_y = if collapsed {
        group_rect.origin.y
    } else {
        group_rect.origin.y + metrics.header_height
    };
    let strip_rect = Rect::new(
        Point::new(group_rect.origin.x, strip_y),
        Size::new(group_rect.size.width, strip_height),
    );
    strip_rect.validate().map_err(ComputeRectsError::Geometry)?;
    output.tab_strips.push(TabStripRect::new(id, strip_rect));

    if !tabs.children.is_empty() {
        let tab_count = tabs.children.len();
        let needs_overflow =
            tab_count as f64 * f64::from(metrics.min_tab_width) > f64::from(group_rect.size.width);

        if !needs_overflow {
            // All tabs remain visible and share the strip width. This naturally
            // compresses headers as more tabs are added, down to min_tab_width.
            let tab_width = group_rect.size.width / tab_count as f32;
            for (index, child_id) in tabs.children.iter().enumerate() {
                if let Some(Node::Panel(panel)) = layout.node(*child_id) {
                    let x = group_rect.origin.x + tab_width * index as f32;
                    let width = if index + 1 == tab_count {
                        (group_rect.origin.x + group_rect.size.width) - x
                    } else {
                        tab_width
                    };
                    let header_rect =
                        Rect::new(Point::new(x, strip_y), Size::new(width, strip_height));
                    output.tab_headers.push(TabHeaderRect::new(
                        id,
                        panel.panel_id,
                        index,
                        header_rect,
                    ));
                }
            }
        } else {
            // This is the project's chosen overflow UX, not a claim about native
            // Photoshop behavior: reserve a trailing button and expose omitted
            // panel ids so the adapter can build a menu/list.
            let requested_button_width =
                metrics.tab_overflow_button_width.min(group_rect.size.width);
            let requested_visible_width = (group_rect.size.width - requested_button_width).max(0.0);
            let visible_count =
                ((requested_visible_width / metrics.min_tab_width).floor() as usize).min(tab_count);
            // When even one minimum-width tab cannot fit beside the button,
            // let the button occupy the whole strip instead of leaving a blank gap.
            let (visible_width, button_width) = if visible_count == 0 {
                (0.0, group_rect.size.width)
            } else {
                (requested_visible_width, requested_button_width)
            };

            let visible_indices: Vec<usize> = if visible_count == 0 {
                Vec::new()
            } else {
                // Keep the active tab visible by selecting a contiguous window
                // that contains it instead of always exposing only the oldest tabs.
                let start = tabs
                    .active_index
                    .saturating_sub(visible_count - 1)
                    .min(tab_count - visible_count);
                (start..start + visible_count).collect()
            };
            if visible_count > 0 {
                let tab_width = visible_width / visible_count as f32;
                for (display_index, child_index) in visible_indices.iter().copied().enumerate() {
                    let child_id = tabs.children[child_index];
                    if let Some(Node::Panel(panel)) = layout.node(child_id) {
                        let x = group_rect.origin.x + tab_width * display_index as f32;
                        let width = if display_index + 1 == visible_count {
                            (group_rect.origin.x + visible_width) - x
                        } else {
                            tab_width
                        };
                        let header_rect =
                            Rect::new(Point::new(x, strip_y), Size::new(width, strip_height));
                        output.tab_headers.push(TabHeaderRect::new(
                            id,
                            panel.panel_id,
                            child_index,
                            header_rect,
                        ));
                    }
                }
            }

            let button_rect = Rect::new(
                Point::new(group_rect.origin.x + visible_width, strip_y),
                Size::new(button_width, strip_height),
            );
            button_rect
                .validate()
                .map_err(ComputeRectsError::Geometry)?;
            let hidden_tabs = tabs
                .children
                .iter()
                .enumerate()
                .filter(|(index, _)| !visible_indices.contains(index))
                .filter_map(|(_, child_id)| match layout.node(*child_id) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !hidden_tabs.is_empty() {
                output
                    .tab_overflows
                    .push(TabOverflowRect::new(id, button_rect, hidden_tabs));
            }
        }
    }

    if collapsed {
        return Ok(());
    }

    let content_y = group_rect.origin.y + metrics.header_height + metrics.tab_strip_height;
    let content_height =
        (group_rect.size.height - metrics.header_height - metrics.tab_strip_height).max(0.0);
    let content_rect = Rect::new(
        Point::new(group_rect.origin.x, content_y),
        Size::new(group_rect.size.width, content_height),
    );
    if let Some(active_child) = tabs.active_child() {
        layout_node(layout, active_child, content_rect, metrics, minimum_sizes, output)?;
    }
    Ok(())
}

fn layout_split(
    layout: &Layout,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    layout_split_inner(layout, id, split, rect, metrics, minimum_sizes, output, false)
}

/// Lays out a floating composition while respecting the independent window title.
///
/// An expanded top group already uses Core's header row for the floating title.
/// A collapsed top group has no such row, so its branch must reserve the same
/// height before allocating group rectangles. Apply this only to composite
/// floating roots; a standalone Tabs root keeps its own established geometry.
fn layout_floating_root(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let title_height = metrics.header_height.min(rect.size.height);
    match layout.node(id) {
        Some(Node::Column(column))
            if column.groups.len() > 1
                && layout.floating_branch_starts_collapsed(id)
                && rect.size.height > title_height =>
        {
            let content_rect = Rect::new(
                Point::new(rect.origin.x, rect.origin.y + title_height),
                Size::new(rect.size.width, rect.size.height - title_height),
            );
            layout_node(layout, id, content_rect, metrics, minimum_sizes, output)
        }
        Some(Node::Split(split))
            if split.axis == Axis::Y
                && layout.floating_branch_starts_collapsed(id)
                && rect.size.height > title_height =>
        {
            let content_rect = Rect::new(
                Point::new(rect.origin.x, rect.origin.y + title_height),
                Size::new(rect.size.width, rect.size.height - title_height),
            );
            layout_node(layout, id, content_rect, metrics, minimum_sizes, output)
        }
        Some(Node::Split(split)) if split.axis == Axis::X => {
            layout_split_inner(layout, id, split, rect, metrics, minimum_sizes, output, true)
        }
        _ => layout_node(layout, id, rect, metrics, minimum_sizes, output),
    }
}



fn layout_split_inner(
    layout: &Layout,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
    reserve_title_for_collapsed_branches: bool,
) -> Result<(), ComputeRectsError> {
    if split.children.is_empty() {
        return Ok(());
    }
    if split.ratios.len() != split.children.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }

    match split.axis {
        Axis::X => {
            let direct_columns = split
                .children
                .iter()
                .all(|child| matches!(layout.node(*child), Some(Node::Column(_))));
            let gap_total =
                metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            let content_width = rect.size.width - gap_total;
            let widths = if direct_columns {
                split
                    .children
                    .iter()
                    .map(|child| preferred_width(layout, *child, metrics, minimum_sizes))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                let minimum_widths = split
                    .children
                    .iter()
                    .map(|child| {
                        let minimum =
                            minimum_node_size(layout, *child, metrics, minimum_sizes)?.width;
                        Ok(minimum.max(metrics.min_column_width))
                    })
                    .collect::<Result<Vec<f32>, ComputeRectsError>>()?;
                weighted_sizes(&split.ratios, content_width, &minimum_widths, Axis::X)?
            };
            let required = widths.iter().sum::<f32>() + gap_total;
            if !required.is_finite() {
                return Err(ComputeRectsError::Geometry(
                    GeometryError::ArithmeticOverflow,
                ));
            }
            if rect.size.width < required {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::X,
                    available: rect.size.width,
                    required,
                });
            }

            let mut x = rect.origin.x;
            for (index, child) in split.children.iter().enumerate() {
                let mut child_rect = Rect::new(
                    Point::new(x, rect.origin.y),
                    Size::new(widths[index], rect.size.height),
                );
                if reserve_title_for_collapsed_branches
                    && layout.floating_branch_starts_collapsed(*child)
                {
                    let title_height = metrics.header_height.min(child_rect.size.height);
                    if child_rect.size.height > title_height {
                        child_rect.origin.y += title_height;
                        child_rect.size.height -= title_height;
                    }
                }
                layout_node(layout, *child, child_rect, metrics, minimum_sizes, output)?;
                x += widths[index];
                if index + 1 < split.children.len() {
                    push_splitter(id, index, Axis::X, x, rect, metrics, output)?;
                    x += metrics.splitter_thickness;
                }
            }
            Ok(())
        }
        Axis::Y => {
            let allocation = allocate_stack_heights(
                layout,
                &split.children,
                &split.ratios,
                rect.size.height,
                Axis::Y,
                metrics,
                minimum_sizes,
            )?;
            if allocation.overflows {
                push_vertical_overflow(id, rect, allocation.content_height, output)?;
            }
            let mut y = rect.origin.y;
            for (index, child) in split.children.iter().enumerate() {
                let child_rect = Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, allocation.sizes[index]),
                );
                layout_node(layout, *child, child_rect, metrics, minimum_sizes, output)?;
                y += allocation.sizes[index];
                if index + 1 < split.children.len() {
                    push_splitter(id, index, Axis::Y, y, rect, metrics, output)?;
                    y += metrics.splitter_thickness;
                }
            }
            Ok(())
        }
    }
}

fn allocate_stack_heights(
    layout: &Layout,
    children: &[NodeId],
    ratios: &[f32],
    available: f32,
    axis: Axis,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<StackAllocation, ComputeRectsError> {
    if ratios.len() != children.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }
    if ratios.is_empty() {
        return Ok(StackAllocation {
            sizes: Vec::new(),
            content_height: 0.0,
            overflows: false,
        });
    }

    let mut collapsed = Vec::with_capacity(children.len());
    let mut minimums = Vec::with_capacity(children.len());
    for child in children {
        #[cfg(test)]
        visit_probe::record_allocate_stack_child(*child);
        collapsed.push(matches!(layout.node(*child), Some(Node::Tabs(tabs)) if tabs.collapsed));
        // Query the common contract for every child; collapsed nodes report
        // their effective strip height from Layout::minimum_node_size too.
        minimums.push(minimum_node_size(layout, *child, metrics, minimum_sizes)?.height);
    }

    let gap_total = metrics.splitter_thickness * children.len().saturating_sub(1) as f32;
    let required = minimums.iter().sum::<f32>() + gap_total;
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }

    // Never shrink a child below its content minimum. When the minimum layout
    // does not fit, return those minimum sizes and let the adapter scroll the
    // resulting content extent.
    if available < required {
        return Ok(StackAllocation {
            sizes: minimums,
            content_height: required,
            overflows: true,
        });
    }

    let normal_weights: Vec<f32> = ratios
        .iter()
        .zip(&collapsed)
        .filter_map(|(ratio, is_collapsed)| if *is_collapsed { None } else { Some(*ratio) })
        .collect();
    let normal_minimums: Vec<f32> = minimums
        .iter()
        .zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { None } else { Some(*minimum) })
        .collect();
    let fixed_collapsed = minimums
        .iter()
        .zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { Some(*minimum) } else { None })
        .sum::<f32>();
    let normal_available = available - gap_total - fixed_collapsed;
    let normal_sizes = weighted_sizes(&normal_weights, normal_available, &normal_minimums, axis)?;

    let mut normal_index = 0;
    let sizes = collapsed
        .iter()
        .enumerate()
        .map(|(index, is_collapsed)| {
            if *is_collapsed {
                minimums[index]
            } else {
                let size = normal_sizes[normal_index];
                normal_index += 1;
                size
            }
        })
        .collect();

    Ok(StackAllocation {
        sizes,
        content_height: available,
        overflows: false,
    })
}

fn push_vertical_overflow(
    owner: NodeId,
    viewport: Rect,
    content_height: f32,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    if !content_height.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    if content_height <= viewport.size.height {
        return Ok(());
    }
    let content_extent = Rect::new(
        viewport.origin,
        Size::new(viewport.size.width, content_height),
    );
    content_extent
        .validate()
        .map_err(ComputeRectsError::Geometry)?;
    output
        .vertical_overflow
        .push(VerticalOverflow::new(owner, viewport, content_extent));
    Ok(())
}

fn append_floating_geometry(output: &mut LayoutRects, floating: LayoutRects) {
    output.columns.extend(floating.columns);
    output.groups.extend(floating.groups);
    output.tab_strips.extend(floating.tab_strips);
    output.tab_headers.extend(floating.tab_headers);
    output.tab_overflows.extend(floating.tab_overflows);
    output.panels.extend(floating.panels);
    output.splitters.extend(floating.splitters);
    output.floating_groups.extend(floating.floating_groups);
    output
        .floating_resize_edges
        .extend(floating.floating_resize_edges);
    output.drop_zones.extend(floating.drop_zones);
    output.vertical_overflow.extend(floating.vertical_overflow);
}

fn minimum_floating_size(
    layout: &Layout,
    root: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Size, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_minimum_node_size_helper(root);
    layout
        .minimum_floating_size_with_cache(root, metrics, minimum_sizes)
        .map_err(|error| match error {
            crate::CommandError::InvalidValue { field: "size", reason }
                if reason.contains("overflow") =>
            {
                ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow)
            }
            _ => ComputeRectsError::InvalidLayoutState,
        })
}

pub(crate) fn clamp_floating_bounds(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    clamp_floating_bounds_with_cache(
        layout,
        root,
        bounds,
        client_area,
        metrics,
        &mut HashMap::new(),
    )
}

fn clamp_floating_bounds_with_cache(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Rect, ComputeRectsError> {
    bounds
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    client_area
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    let minimum = minimum_floating_size(layout, root, metrics, minimum_sizes)?;
    if minimum.width > client_area.size.width {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: client_area.size.width,
            required: minimum.width,
        });
    }
    if minimum.height > client_area.size.height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::Y,
            available: client_area.size.height,
            required: minimum.height,
        });
    }

    // This helper does not mutate the model. Geometry snapshots may use the
    // returned rectangle transiently; workspace recovery may commit it after
    // reporting the change. Oversized windows are fit only when the subtree's
    // minimum size remains satisfied.
    let width = bounds
        .size
        .width
        .max(minimum.width)
        .min(client_area.size.width);
    let height = bounds
        .size
        .height
        .max(minimum.height)
        .min(client_area.size.height);
    let client_right = client_area
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let client_bottom = client_area
        .checked_bottom()
        .map_err(ComputeRectsError::Geometry)?;
    if client_right <= client_area.origin.x || client_bottom <= client_area.origin.y {
        return Err(ComputeRectsError::Geometry(
            GeometryError::NonPositiveDimension,
        ));
    }
    // Compute the remaining extent before adding it to the origin. Subtracting
    // a large window size from a rounded far edge can otherwise round the
    // minimum origin one representable f32 step outside the client area.
    let max_x = client_area.origin.x + (client_area.size.width - width);
    let max_y = client_area.origin.y + (client_area.size.height - height);
    if !max_x.is_finite()
        || !max_y.is_finite()
        || max_x < client_area.origin.x
        || max_y < client_area.origin.y
    {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    let origin = Point::new(
        bounds.origin.x.clamp(client_area.origin.x, max_x),
        bounds.origin.y.clamp(client_area.origin.y, max_y),
    );
    let result = Rect::new(origin, Size::new(width, height));
    result
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

fn weighted_sizes(
    weights: &[f32],
    available: f32,
    minimums: &[f32],
    axis: Axis,
) -> Result<Vec<f32>, ComputeRectsError> {
    if weights.len() != minimums.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }
    if weights.is_empty() {
        return Ok(Vec::new());
    }
    if !available.is_finite()
        || minimums
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(ComputeRectsError::Geometry(GeometryError::NonFinite));
    }
    let required = minimums.iter().sum::<f32>();
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    if available < required {
        return Err(ComputeRectsError::InsufficientSpace {
            axis,
            available,
            required,
        });
    }

    // Collapsed children keep their fixed minimum height and are excluded
    // before this function is called. If that leaves only zero-weight expanded
    // children, use equal weights so a valid parent ratio vector still fills the
    // available stack space. Positive weights retain their original proportions.
    let equal_weights;
    let weights = if weights.iter().all(|weight| *weight == 0.0) {
        equal_weights = vec![1.0; weights.len()];
        equal_weights.as_slice()
    } else {
        weights
    };

    let mut output = vec![0.0; weights.len()];
    let mut active: Vec<usize> = (0..weights.len()).collect();
    let mut remaining = available;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        if !weight_sum.is_finite() || weight_sum <= 0.0 {
            return Err(ComputeRectsError::InvalidLayoutState);
        }
        let undersized = active
            .iter()
            .copied()
            .find(|index| remaining * weights[*index] / weight_sum < minimums[*index]);
        if let Some(index) = undersized {
            output[index] = minimums[index];
            remaining -= minimums[index];
            active.retain(|candidate| *candidate != index);
        } else {
            let last_active_position = active.len().saturating_sub(1);
            let mut assigned = 0.0_f32;
            for (position, index) in active.iter().copied().enumerate() {
                let size = if position == last_active_position {
                    remaining - assigned
                } else {
                    let size = remaining * weights[index] / weight_sum;
                    assigned += size;
                    size
                };
                output[index] = size;
            }
            break;
        }
    }
    Ok(output)
}

fn push_splitter(
    owner: NodeId,
    boundary_index: usize,
    axis: Axis,
    boundary: f32,
    parent_rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let (cross_size, cross_origin) = match axis {
        Axis::X => (parent_rect.size.height, parent_rect.origin.y),
        Axis::Y => (parent_rect.size.width, parent_rect.origin.x),
    };
    let rect = match axis {
        Axis::X => Rect::new(
            Point::new(
                boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0,
                cross_origin,
            ),
            Size::new(metrics.splitter_hit_zone, cross_size),
        ),
        Axis::Y => Rect::new(
            Point::new(
                cross_origin,
                boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0,
            ),
            Size::new(cross_size, metrics.splitter_hit_zone),
        ),
    };
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    output
        .splitters
        .push(SplitterRect::new(owner, boundary_index, axis, rect));
    Ok(())
}

fn push_resize_edges(
    group: crate::FloatingGroupId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let width = metrics.splitter_hit_zone.min(rect.size.width);
    let height = metrics.splitter_hit_zone.min(rect.size.height);
    let left = rect.origin.x;
    let right = rect.origin.x + rect.size.width - width;
    let top = rect.origin.y;
    let bottom = rect.origin.y + rect.size.height - height;
    let right_edge = rect.origin.x + rect.size.width;
    let bottom_edge = rect.origin.y + rect.size.height;
    // Corners precede edges so an adapter can prioritize corner resizing when
    // the hit regions overlap.
    let edges = [
        (
            ResizeEdge::TopLeft,
            Rect::new(Point::new(left, top), Size::new(width, height)),
        ),
        (
            ResizeEdge::TopRight,
            Rect::new(Point::new(right, top), Size::new(width, height)),
        ),
        (
            ResizeEdge::BottomLeft,
            Rect::new(Point::new(left, bottom), Size::new(width, height)),
        ),
        (
            ResizeEdge::BottomRight,
            Rect::new(Point::new(right, bottom), Size::new(width, height)),
        ),
        (
            ResizeEdge::Left,
            Rect::new(Point::new(left, top), Size::new(width, rect.size.height)),
        ),
        (
            ResizeEdge::Right,
            Rect::new(Point::new(right, top), Size::new(width, rect.size.height)),
        ),
        (
            ResizeEdge::Top,
            Rect::new(Point::new(left, top), Size::new(rect.size.width, height)),
        ),
        (
            ResizeEdge::Bottom,
            Rect::new(Point::new(left, bottom), Size::new(rect.size.width, height)),
        ),
    ];
    // Keep checked edge calculations explicit so arithmetic overflow is rejected.
    if !right_edge.is_finite() || !bottom_edge.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    for (edge, edge_rect) in edges {
        edge_rect.validate().map_err(ComputeRectsError::Geometry)?;
        output
            .floating_resize_edges
            .push(FloatingResizeEdgeRect::new(group, edge, edge_rect));
    }
    Ok(())
}

#[cfg(test)]
#[path = "compute_rects/tests.rs"]
mod tests;
