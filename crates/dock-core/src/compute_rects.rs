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
            .map_err(|_| ComputeRectsError::InvalidMetrics)?;
        viewport
            .validate_non_empty()
            .map_err(ComputeRectsError::Geometry)?;
        self.validate(metrics)
            .map_err(|_| ComputeRectsError::InvalidLayout)?;

        // NodeId is a sufficient key only inside this pass: both the Layout tree
        // and DockMetrics are fixed until the returned geometry snapshot is complete.
        // Never retain this cache across compute_rects calls.
        let mut minimum_size_cache = HashMap::new();

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
            Some(root) => preferred_width(self, root, metrics, &mut minimum_size_cache)?,
            None => 0.0,
        };
        let right_width = match right_root {
            Some(root) => preferred_width(self, root, metrics, &mut minimum_size_cache)?,
            None => 0.0,
        };

        let available_canvas = viewport.size.width - left_width - right_width;
        if !available_canvas.is_finite() {
            return Err(ComputeRectsError::Geometry(
                GeometryError::ArithmeticOverflow,
            ));
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
            layout_node(self, root, area, metrics, &mut output, &mut minimum_size_cache)?;
        }
        if let Some(root) = right_root {
            let area = Rect::new(
                Point::new(right_x, viewport.origin.y),
                Size::new(right_width, viewport.size.height),
            );
            layout_node(self, root, area, metrics, &mut output, &mut minimum_size_cache)?;
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
            for (id, floating) in self.floating_groups_in_paint_order() {
                let area = self
                    .dock_area(floating.area)
                    .ok_or(ComputeRectsError::InvalidLayout)?;
                let bounds = clamp_floating_bounds_with_cache(
                    self,
                    area.root,
                    floating.bounds,
                    viewport,
                    metrics,
                    &mut minimum_size_cache,
                )?;
                output
                    .floating_groups
                    .push(FloatingGroupRect::new(id, bounds, floating.z_order));
                push_resize_edges(id, bounds, metrics, &mut output)?;
                layout_node(self, area.root, bounds, metrics, &mut output, &mut minimum_size_cache)?;
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
        .ok_or(ComputeRectsError::InvalidLayout)?;
    let canvas = output.canvas.ok_or(ComputeRectsError::InvalidLayout)?;
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
        return Err(ComputeRectsError::InvalidLayout);
    };

    let instance = layout
        .panel_instance(flyout.panel)
        .ok_or(ComputeRectsError::InvalidLayout)?;
    let constraints = instance.constraints;
    let min_width = constraints.min_size.width;
    let preferred_width = instance
        .state
        .last_known_size()
        .map(|size| size.width)
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
        _ => return Err(ComputeRectsError::InvalidLayout),
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
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<f32, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_preferred_width(id);
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayout)?;
    let width = match node {
        Node::Column(column) => effective_column_width(column, metrics),
        Node::Tabs(_) | Node::Panel(_) => metrics
            .default_column_width
            .max(minimum_node_size(layout, id, metrics, minimum_size_cache)?.width),
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                width += preferred_width(layout, *child, metrics, minimum_size_cache)?;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            width
        }
        Node::Split(split) => {
            let mut width = 0.0_f32;
            for child in &split.children {
                width = width.max(preferred_width(layout, *child, metrics, minimum_size_cache)?);
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
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<Size, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_minimum_node_size_helper(id);
    layout
        .minimum_node_size_with_cache(id, metrics, minimum_size_cache)
        .map_err(|error| match error {
            crate::CommandError::InvalidValue { field: "size", .. } => {
                ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow)
            }
            _ => ComputeRectsError::InvalidLayout,
        })
}

fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayout)?;

    // Columns and vertical splits manage their own stack overflow. Other nodes
    // need an expanded content rect when the parent viewport is shorter than
    // their minimum geometry, with a separate viewport/content extent record.
    let manages_vertical_stack = matches!(node, Node::Column(_))
        || matches!(node, Node::Split(split) if split.axis == Axis::Y);
    let mut content_rect = rect;
    if !manages_vertical_stack {
        let minimum = minimum_node_size(layout, id, metrics, minimum_size_cache)?;
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
            output,
            minimum_size_cache,
        ),
        Node::Tabs(tabs) => layout_tabs(
            layout,
            id,
            tabs,
            content_rect,
            metrics,
            output,
            minimum_size_cache,
        ),
        Node::Panel(panel) => {
            let minimum = layout
                .panel_instance(panel.panel_id)
                .ok_or(ComputeRectsError::InvalidLayout)?
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
            output,
            minimum_size_cache,
        ),
    }
}

fn layout_column(
    layout: &Layout,
    id: NodeId,
    column: &Column,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
    minimum_size_cache: &mut HashMap<NodeId, Size>,
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
            .map_err(|_| ComputeRectsError::InvalidLayout)?;
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
        minimum_size_cache,
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
        layout_node(layout, *group_id, group_rect, metrics, output, minimum_size_cache)?;
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
    output: &mut LayoutRects,
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<(), ComputeRectsError> {
    let collapsed = tabs.collapsed;
    let minimum_size = minimum_node_size(layout, id, metrics, minimum_size_cache)?;
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

            if visible_count > 0 {
                let tab_width = visible_width / visible_count as f32;
                for (index, child_id) in tabs.children.iter().take(visible_count).enumerate() {
                    if let Some(Node::Panel(panel)) = layout.node(*child_id) {
                        let x = group_rect.origin.x + tab_width * index as f32;
                        let width = if index + 1 == visible_count {
                            (group_rect.origin.x + visible_width) - x
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
                .skip(visible_count)
                .filter_map(|child_id| match layout.node(*child_id) {
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
        layout_node(layout, active_child, content_rect, metrics, output, minimum_size_cache)?;
    }
    Ok(())
}

fn layout_split(
    layout: &Layout,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<(), ComputeRectsError> {
    if split.children.is_empty() {
        return Ok(());
    }
    if split.ratios.len() != split.children.len() {
        return Err(ComputeRectsError::InvalidLayout);
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
                    .map(|child| preferred_width(layout, *child, metrics, minimum_size_cache))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                let minimum_widths = split
                    .children
                    .iter()
                    .map(|child| {
                        let minimum = minimum_node_size(layout, *child, metrics, minimum_size_cache)?.width;
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
                let child_rect = Rect::new(
                    Point::new(x, rect.origin.y),
                    Size::new(widths[index], rect.size.height),
                );
                layout_node(layout, *child, child_rect, metrics, output, minimum_size_cache)?;
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
                minimum_size_cache,
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
                layout_node(layout, *child, child_rect, metrics, output, minimum_size_cache)?;
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
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<StackAllocation, ComputeRectsError> {
    if ratios.len() != children.len() {
        return Err(ComputeRectsError::InvalidLayout);
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
        minimums.push(minimum_node_size(layout, *child, metrics, minimum_size_cache)?.height);
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

pub(crate) fn clamp_floating_bounds(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let mut minimum_size_cache = HashMap::new();
    clamp_floating_bounds_with_cache(
        layout,
        root,
        bounds,
        client_area,
        metrics,
        &mut minimum_size_cache,
    )
}

fn clamp_floating_bounds_with_cache(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
    minimum_size_cache: &mut HashMap<NodeId, Size>,
) -> Result<Rect, ComputeRectsError> {
    bounds
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    client_area
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    let minimum = minimum_node_size(layout, root, metrics, minimum_size_cache)?;
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
    if width < minimum.width || height < minimum.height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: if width < minimum.width {
                Axis::X
            } else {
                Axis::Y
            },
            available: if width < minimum.width { width } else { height },
            required: if width < minimum.width {
                minimum.width
            } else {
                minimum.height
            },
        });
    }
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
    let max_x = client_right - width;
    let max_y = client_bottom - height;
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
        return Err(ComputeRectsError::InvalidLayout);
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
            return Err(ComputeRectsError::InvalidLayout);
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
            for index in active {
                output[index] = remaining * weights[index] / weight_sum;
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
mod tests {
    use super::{clamp_floating_bounds, ComputeRectsError};
    use crate::layout_builder::LayoutBuilder;
    use crate::{
        Axis, ColumnMode, DockMetrics, DockSide, GeometryError, LayoutRects, Node,
        PanelConstraints, PanelKey, Point, Profile, Rect, Size,
    };

    const PIXEL_TOLERANCE: f32 = 1.0e-3;

    fn constraints() -> PanelConstraints {
        PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        )
    }

    fn key(value: &str) -> PanelKey {
        PanelKey::new(value).unwrap()
    }

    fn viewport(width: f32, height: f32) -> Rect {
        Rect::new(Point::ZERO, Size::new(width, height))
    }

    // Test-only stand-in for the renderer's final logical->physical pixel snap.
    // Production dock-core geometry deliberately does not accept a scale factor.
    fn snap_rect_at_render_boundary(rect: Rect, scale: f32) -> Rect {
        assert!(scale.is_finite() && scale > 0.0);
        let snap = |coordinate: f32| (coordinate * scale).round() / scale;
        let left = snap(rect.origin.x);
        let top = snap(rect.origin.y);
        let right = snap(rect.origin.x + rect.size.width);
        let bottom = snap(rect.origin.y + rect.size.height);
        Rect::new(Point::new(left, top), Size::new(right - left, bottom - top))
    }

    #[test]
    fn columns_keep_model_width_and_canvas_receives_horizontal_remainder() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, left_panel_node) = builder.panel(key("rect-left"), constraints());
        let left_group = builder.tabs(vec![left_panel_node], 0, false);
        let left_column = builder.column(240.0, ColumnMode::Expanded, vec![left_group], vec![1.0]);

        let (_, inner_panel_node) = builder.panel(key("rect-inner"), constraints());
        let inner_group = builder.tabs(vec![inner_panel_node], 0, false);
        let inner_column =
            builder.column(300.0, ColumnMode::Expanded, vec![inner_group], vec![1.0]);
        let left_root = builder.split(Axis::X, vec![left_column, inner_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, left_root);

        let (_, right_panel_node) = builder.panel(key("rect-right"), constraints());
        let right_group = builder.tabs(vec![right_panel_node], 0, false);
        let right_column =
            builder.column(220.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
        builder.main_area(DockSide::Right, right_column);

        let layout = builder.build();
        let first = layout
            .compute_rects(viewport(1200.0, 700.0), &metrics)
            .unwrap();
        let second = layout
            .compute_rects(viewport(1300.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(first.columns[0].rect.size.width, 240.0);
        assert_eq!(first.columns[1].rect.size.width, 300.0);
        assert_eq!(first.columns[2].rect.size.width, 220.0);
        assert_eq!(first.columns[0].rect.origin.x, 0.0);
        assert_eq!(first.columns[1].rect.origin.x, 244.0);
        assert_eq!(first.columns[2].rect.origin.x, 980.0);
        assert_eq!(first.canvas.unwrap().origin.x, 544.0);
        assert_eq!(first.canvas.unwrap().size.width, 436.0);
        assert_eq!(second.columns[0].rect.size.width, 240.0);
        assert_eq!(second.columns[1].rect.size.width, 300.0);
        assert_eq!(second.columns[2].rect.size.width, 220.0);
        assert_eq!(second.canvas.unwrap().size.width, 536.0);
        assert_eq!(first.canvas.unwrap().size.height, 700.0);
        assert_eq!(second.canvas.unwrap().size.height, 500.0);
        assert_eq!(first.splitters.len(), 1);
    }

    #[test]
    fn collapsed_groups_keep_fixed_height_and_are_excluded_from_ratios() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for (index, collapsed) in [(0, true), (1, false), (2, false)] {
            let (_, panel_node) = builder.panel(key(&format!("ratio-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, collapsed));
        }
        let column = builder.column(
            280.0,
            ColumnMode::Expanded,
            groups.clone(),
            vec![0.2, 0.3, 0.5],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let heights: Vec<_> = rects
            .groups
            .iter()
            .map(|group| group.rect.size.height)
            .collect();
        assert_eq!(heights[0], metrics.collapsed_group_height);
        assert!((heights[1] / heights[2] - 0.3 / 0.5).abs() < PIXEL_TOLERANCE);
        assert!(
            (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
                < PIXEL_TOLERANCE
        );
        assert_eq!(rects.panels.len(), 2);
        assert!(
            rects
                .panels
                .iter()
                .all(|panel| panel.rect.size.height > 0.0)
        );
    }

    #[test]
    fn ratio_allocation_clamps_small_group_to_its_panel_minimum_and_redistributes_remainder() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for (index, collapsed) in [(0, true), (1, false), (2, false)] {
            let (_, panel_node) = builder.panel(key(&format!("floor-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, collapsed));
        }
        let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.8, 0.19, 0.01]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let heights: Vec<_> = rects
            .groups
            .iter()
            .map(|group| group.rect.size.height)
            .collect();
        let minimum =
            metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;

        assert_eq!(heights[0], metrics.collapsed_group_height);
        assert!((heights[2] - minimum).abs() < PIXEL_TOLERANCE);
        assert!((heights[1] - 348.0).abs() < PIXEL_TOLERANCE);
        assert!(heights[2] >= minimum);
        assert!(
            (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
                < PIXEL_TOLERANCE
        );
    }

    #[test]
    fn active_tab_gets_panel_bounds_and_all_tabs_get_header_rectangles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (first_panel, first_node) = builder.panel(key("tab-first"), constraints());
        let (active_panel, active_node) = builder.panel(key("tab-active"), constraints());
        let group = builder.tabs(vec![first_node, active_node], 1, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(rects.tab_headers.len(), 2);
        assert_eq!(rects.tab_headers[0].panel, first_panel);
        assert_eq!(rects.tab_headers[1].panel, active_panel);
        assert_eq!(rects.tab_strips[0].group, group);
        assert_eq!(rects.panels.len(), 1);
        assert_eq!(rects.panels[0].panel, active_panel);
        assert_eq!(
            rects.panels[0].rect.origin.y,
            metrics.header_height + metrics.tab_strip_height
        );
        assert!(rects.drop_zones.is_empty());
    }

    #[test]
    fn floating_geometry_preserves_bounds_z_order_and_resize_handles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("floating-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let model_bounds = Rect::new(Point::new(-30.0, 45.0), Size::new(360.0, 260.0));
        let floating = builder.floating_group(group, model_bounds, 9);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 700.0), &metrics)
            .unwrap();

        let expected_bounds = Rect::new(Point::new(0.0, 45.0), Size::new(360.0, 260.0));
        assert_eq!(rects.floating_groups.len(), 1);
        assert_eq!(rects.floating_groups[0].group, floating);
        assert_eq!(rects.floating_groups[0].rect, expected_bounds);
        assert_eq!(rects.floating_groups[0].z_order, 9);
        assert_eq!(rects.floating_resize_edges.len(), 8);
        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(rects.panels[0].rect.origin.x, expected_bounds.origin.x);
        assert_eq!(
            rects.panels[0].rect.origin.y,
            expected_bounds.origin.y + metrics.header_height + metrics.tab_strip_height
        );
        // Viewport clamping applies to geometry output, not persistent model bounds.
        assert_eq!(
            layout.floating_group(floating).unwrap().bounds,
            model_bounds
        );
    }

    #[test]
    fn floating_collapsed_group_stays_strip_sized_inside_window_bounds() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("floating-collapsed"), constraints());
        let group = builder.tabs(vec![panel_node], 0, true);
        let bounds = Rect::new(Point::new(20.0, 30.0), Size::new(360.0, 260.0));
        builder.floating_group(group, bounds, 2);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 700.0), &metrics)
            .unwrap();

        assert_eq!(rects.floating_groups[0].rect, bounds);
        assert_eq!(
            rects.groups[0].rect.size.height,
            metrics.collapsed_group_height
        );
        assert_eq!(
            rects.tab_strips[0].rect.size.height,
            metrics.collapsed_group_height
        );
        assert!(rects.panels.is_empty());
        assert!(layout.panel_instance(panel).is_some());
    }

    #[test]
    fn icon_rail_uses_rail_width_and_emits_flattened_panel_targets() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("icon-rail-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(
            metrics.rail_width,
            ColumnMode::IconsOnly,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(rects.columns[0].rect.size.width, metrics.rail_width);
        assert_eq!(
            rects.groups[0].rect.size.height,
            metrics.collapsed_group_height
        );
        assert!(rects.tab_strips.is_empty());
        assert_eq!(rects.tab_headers.len(), 1);
        assert_eq!(rects.tab_headers[0].panel, panel);
        assert_eq!(rects.tab_headers[0].index, 0);
        assert_eq!(
            rects.tab_headers[0].rect.size.height,
            metrics.collapsed_group_height
        );
        assert!(rects.panels.is_empty());
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(column))
                if column.mode == ColumnMode::IconsOnly && column.width == metrics.rail_width
        ));
    }

    #[test]
    fn golden_rects_for_single_left_column_are_exact_and_deterministic() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("golden-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let viewport = viewport(1000.0, 600.0);

        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(layout.compute_rects(viewport, &metrics).unwrap(), rects);

        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 600.0)))
        );
        assert_eq!(rects.columns.len(), 1);
        assert_eq!(
            rects.columns[0].rect,
            Rect::new(Point::ZERO, Size::new(280.0, 600.0))
        );
        assert_eq!(rects.groups.len(), 1);
        assert_eq!(
            rects.groups[0].rect,
            Rect::new(Point::ZERO, Size::new(280.0, 600.0))
        );
        assert_eq!(rects.tab_strips.len(), 1);
        assert_eq!(
            rects.tab_strips[0].rect,
            Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
        );
        assert_eq!(rects.tab_headers.len(), 1);
        assert_eq!(rects.tab_headers[0].panel, panel);
        assert_eq!(rects.tab_headers[0].index, 0);
        assert_eq!(
            rects.tab_headers[0].rect,
            Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
        );
        assert_eq!(rects.panels.len(), 1);
        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(
            rects.panels[0].rect,
            Rect::new(Point::new(0.0, 56.0), Size::new(280.0, 544.0))
        );
        assert!(rects.splitters.is_empty());
        assert!(rects.drop_zones.is_empty());
        assert!(rects.vertical_overflow.is_empty());
        assert!(layout.node(column).is_some());
        assert!(layout.node(group).is_some());
    }

    #[test]
    fn golden_rects_for_1280_by_800_cover_both_docks_columns_groups_and_splitters() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, upper_panel_node) = builder.panel(key("golden-1280-left-upper"), constraints());
        let upper_group = builder.tabs(vec![upper_panel_node], 0, false);
        let (_, lower_panel_node) = builder.panel(key("golden-1280-left-lower"), constraints());
        let lower_group = builder.tabs(vec![lower_panel_node], 0, false);
        let first_column = builder.column(
            240.0,
            ColumnMode::Expanded,
            vec![upper_group, lower_group],
            vec![0.5, 0.5],
        );

        let (_, middle_panel_node) = builder.panel(key("golden-1280-left-middle"), constraints());
        let middle_group = builder.tabs(vec![middle_panel_node], 0, false);
        let second_column =
            builder.column(300.0, ColumnMode::Expanded, vec![middle_group], vec![1.0]);
        let left_root = builder.split(Axis::X, vec![first_column, second_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, left_root);

        let (_, right_panel_node) = builder.panel(key("golden-1280-right"), constraints());
        let right_group = builder.tabs(vec![right_panel_node], 0, false);
        let right_column =
            builder.column(220.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
        builder.main_area(DockSide::Right, right_column);

        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1280.0, 800.0), &metrics)
            .unwrap();

        // Manual golden values: left dock = 240 + 4 + 300 = 544 px;
        // right dock = 220 px; Canvas gets the exact 516 px remainder.
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(544.0, 0.0), Size::new(516.0, 800.0)))
        );
        assert_eq!(rects.columns.len(), 3);
        assert_eq!(
            rects
                .columns
                .iter()
                .map(|item| item.rect)
                .collect::<Vec<_>>(),
            vec![
                Rect::new(Point::ZERO, Size::new(240.0, 800.0)),
                Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
                Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
            ]
        );

        assert_eq!(rects.groups.len(), 4);
        assert_eq!(
            rects
                .groups
                .iter()
                .map(|item| item.rect)
                .collect::<Vec<_>>(),
            vec![
                Rect::new(Point::ZERO, Size::new(240.0, 398.0)),
                Rect::new(Point::new(0.0, 402.0), Size::new(240.0, 398.0)),
                Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
                Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
            ]
        );

        // Hit rectangles intentionally use the 8 px splitter hit zone,
        // centered around each 4 px visual splitter.
        assert_eq!(rects.splitters.len(), 2);
        assert_eq!(rects.splitters[0].owner, first_column);
        assert_eq!(rects.splitters[0].boundary_index, 0);
        assert_eq!(rects.splitters[0].axis, Axis::Y);
        assert_eq!(
            rects.splitters[0].rect,
            Rect::new(Point::new(0.0, 396.0), Size::new(240.0, 8.0))
        );
        assert_eq!(rects.splitters[1].owner, left_root);
        assert_eq!(rects.splitters[1].boundary_index, 0);
        assert_eq!(rects.splitters[1].axis, Axis::X);
        assert_eq!(
            rects.splitters[1].rect,
            Rect::new(Point::new(238.0, 0.0), Size::new(8.0, 800.0))
        );

        assert_eq!(rects.panels.len(), 4);
        assert_eq!(
            rects
                .panels
                .iter()
                .map(|item| item.rect)
                .collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 56.0), Size::new(240.0, 342.0)),
                Rect::new(Point::new(0.0, 458.0), Size::new(240.0, 342.0)),
                Rect::new(Point::new(244.0, 56.0), Size::new(300.0, 744.0)),
                Rect::new(Point::new(1060.0, 56.0), Size::new(220.0, 744.0)),
            ]
        );
        assert!(rects.vertical_overflow.is_empty());
        assert!(rects.drop_zones.is_empty());
    }

    #[test]
    fn fractional_logical_viewport_preserves_dpi_independent_geometry_and_reports_small_height_overflow()
     {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("fractional-logical-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(200.25, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        // Geometry remains in logical px: non-integral viewport origins, widths,
        // and heights are preserved exactly instead of rounded for a display DPI.
        let viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 196.5));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(212.5, 7.5), Size::new(320.5, 196.5),))
        );
        assert_eq!(
            rects.columns[0].rect,
            Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 196.5),)
        );
        assert_eq!(
            rects.groups[0].rect,
            Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 196.5),)
        );
        assert_eq!(
            rects.panels[0].rect,
            Rect::new(Point::new(12.25, 63.5), Size::new(200.25, 140.5),)
        );
        assert!(rects.vertical_overflow.is_empty());

        // The panel's declared minimum height is 60 px, so the expanded group
        // minimum is header 28 + tab strip 28 + content 60 = 116 logical px.
        // A viewport 0.5 px below that minimum must report overflow.
        let short_viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 115.5));
        let short_rects = layout.compute_rects(short_viewport, &metrics).unwrap();
        assert_eq!(short_rects.vertical_overflow.len(), 1);
        assert_eq!(short_rects.vertical_overflow[0].owner, column);
        assert_eq!(
            short_rects.vertical_overflow[0].viewport,
            Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 115.5),)
        );
        assert_eq!(
            short_rects.vertical_overflow[0].content_extent,
            Rect::new(Point::new(12.25, 7.5), Size::new(200.25, 116.0),)
        );
        assert_eq!(short_rects.groups[0].rect.size.height, 116.0);
        assert_eq!(short_rects.panels[0].rect.size.height, 60.0);

        // Crossing the minimum Canvas width by 0.25 logical px must fail,
        // rather than shrinking either dock or the Canvas.
        let too_narrow = Rect::new(Point::new(12.25, 7.5), Size::new(520.0, 196.5));
        assert!(matches!(
            layout.compute_rects(too_narrow, &metrics),
            Err(ComputeRectsError::InsufficientCanvasWidth { available, minimum })
                if available == 319.75 && minimum == metrics.min_canvas_width
        ));
    }

    #[test]
    fn pixel_snapping_is_deferred_to_the_render_boundary_for_common_dpi_scales() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("dpi-boundary-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(200.25, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let viewport = Rect::new(Point::new(12.25, 7.5), Size::new(520.75, 196.5));
        let expected_logical_panel = Rect::new(Point::new(12.25, 63.5), Size::new(200.25, 140.5));
        let baseline = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(baseline.panels[0].rect, expected_logical_panel);

        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            // Recomputing core geometry must return identical logical-pixel data.
            let rects = layout.compute_rects(viewport, &metrics).unwrap();
            assert_eq!(rects, baseline, "logical geometry changed at scale {scale}");
            let logical_panel = rects.panels[0].rect;
            assert_eq!(logical_panel, expected_logical_panel);

            // Snapping is a separate operation on a copy at the render boundary.
            let snapped = snap_rect_at_render_boundary(logical_panel, scale);
            assert_ne!(
                snapped, logical_panel,
                "fractional edges should snap at scale {scale}"
            );
            let physical_edges = [
                snapped.origin.x * scale,
                snapped.origin.y * scale,
                (snapped.origin.x + snapped.size.width) * scale,
                (snapped.origin.y + snapped.size.height) * scale,
            ];
            for edge in physical_edges {
                assert!(
                    (edge - edge.round()).abs() < PIXEL_TOLERANCE,
                    "edge {edge} is not pixel-aligned at scale {scale}"
                );
            }

            // Applying render snapping cannot mutate the geometry snapshot.
            assert_eq!(rects.panels[0].rect, expected_logical_panel);
        }
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn configurable_metrics_drive_header_strip_and_splitter_geometry() {
        let metrics = DockMetrics {
            header_height: 31.5,
            tab_strip_height: 20.25,
            splitter_thickness: 3.5,
            splitter_hit_zone: 9.0,
            ..DockMetrics::default()
        };
        assert_eq!(metrics.validate(), Ok(()));

        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for name in ["custom-metrics-upper", "custom-metrics-lower"] {
            let (_, panel_node) = builder.panel(key(name), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, false));
        }
        let column = builder.column(280.0, ColumnMode::Expanded, groups.clone(), vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            rects
                .tab_strips
                .iter()
                .map(|item| item.rect)
                .collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 31.5), Size::new(280.0, 20.25)),
                Rect::new(Point::new(0.0, 283.25), Size::new(280.0, 20.25)),
            ]
        );
        assert_eq!(
            rects
                .panels
                .iter()
                .map(|item| item.rect)
                .collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 51.75), Size::new(280.0, 196.5)),
                Rect::new(Point::new(0.0, 303.5), Size::new(280.0, 196.5)),
            ]
        );
        assert_eq!(rects.splitters.len(), 1);
        assert_eq!(rects.splitters[0].owner, column);
        assert_eq!(
            rects.splitters[0].rect,
            Rect::new(Point::new(0.0, 245.5), Size::new(280.0, 9.0))
        );
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 500.0)))
        );

        // The model remains unchanged; these configurable presentation metrics
        // affect only the computed geometry snapshot.
        assert!(layout.node(column).is_some());
        assert_eq!(metrics.header_height, 31.5);
        assert_eq!(metrics.tab_strip_height, 20.25);
        assert_eq!(metrics.splitter_thickness, 3.5);
        assert_eq!(metrics.splitter_hit_zone, 9.0);
    }

    #[test]
    fn floating_bounds_clamp_to_client_area_respect_minimums_and_sort_by_z() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut ids = Vec::new();
        for (name, z_order, origin, size) in [
            (
                "float-high",
                3,
                Point::new(-100.0, 650.0),
                Size::new(420.0, 360.0),
            ),
            (
                "float-low",
                1,
                Point::new(900.0, -50.0),
                Size::new(420.0, 360.0),
            ),
            (
                "float-mid",
                2,
                Point::new(30.0, 40.0),
                Size::new(100.0, 80.0),
            ),
        ] {
            let constraints = if name == "float-mid" {
                PanelConstraints::new(
                    Size::new(160.0, 100.0),
                    Size::new(180.0, 140.0),
                    Some(Size::new(800.0, 600.0)),
                    false,
                )
            } else {
                constraints()
            };
            let (_, panel_node) = builder.panel(key(name), constraints);
            let group = builder.tabs(vec![panel_node], 0, false);
            ids.push(builder.floating_group(group, Rect::new(origin, size), z_order));
        }
        let layout = builder.build();
        let client = Rect::new(Point::new(10.0, 20.0), Size::new(1000.0, 700.0));

        let rects = layout.compute_rects(client, &metrics).unwrap();

        assert_eq!(
            rects
                .floating_groups
                .iter()
                .map(|item| item.z_order)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(rects.floating_groups[0].group, ids[1]);
        assert_eq!(
            rects.floating_groups[0].rect,
            Rect::new(Point::new(590.0, 20.0), Size::new(420.0, 360.0))
        );
        assert_eq!(rects.floating_groups[1].group, ids[2]);
        assert_eq!(
            rects.floating_groups[1].rect,
            Rect::new(Point::new(30.0, 40.0), Size::new(160.0, 156.0))
        );
        assert_eq!(rects.floating_groups[2].group, ids[0]);
        assert_eq!(
            rects.floating_groups[2].rect,
            Rect::new(Point::new(10.0, 360.0), Size::new(420.0, 360.0))
        );
        let small_panel = layout.floating_group(ids[2]).unwrap().bounds;
        assert_eq!(small_panel.size, Size::new(100.0, 80.0));
        assert_eq!(
            layout.floating_group(ids[0]).unwrap().bounds.origin,
            Point::new(-100.0, 650.0)
        );
        assert_eq!(rects.floating_resize_edges.len(), 24);
        assert!(rects.vertical_overflow.is_empty());
    }

    #[test]
    fn vertical_split_reports_content_extent_when_child_minimums_do_not_fit() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut children = Vec::new();
        for name in ["split-top", "split-bottom"] {
            let (_, panel_node) = builder.panel(key(name), constraints());
            children.push(builder.tabs(vec![panel_node], 0, false));
        }
        let root = builder.split(Axis::Y, children, vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, root);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 120.0), &metrics)
            .unwrap();

        assert_eq!(rects.vertical_overflow.len(), 1);
        assert_eq!(rects.vertical_overflow[0].owner, root);
        assert_eq!(rects.vertical_overflow[0].viewport.size.height, 120.0);
        assert_eq!(rects.vertical_overflow[0].content_extent.size.height, 236.0);
        assert_eq!(rects.panels.len(), 2);
        assert!(
            rects
                .panels
                .iter()
                .all(|panel| panel.rect.size.height == 60.0)
        );
    }

    #[test]
    fn non_finite_viewport_is_rejected_without_emitting_non_finite_rectangles() {
        let layout = LayoutBuilder::new(Profile::Photoshop).build();
        let metrics = DockMetrics::default();
        for viewport in [
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(1000.0, 700.0)),
            Rect::new(Point::ZERO, Size::new(f32::INFINITY, 700.0)),
            Rect::new(Point::ZERO, Size::new(1000.0, f32::INFINITY)),
        ] {
            assert!(matches!(
                layout.compute_rects(viewport, &metrics),
                Err(ComputeRectsError::Geometry(_))
            ));
        }
    }

    #[test]
    fn floating_minimum_must_fit_client_height() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let tight = PanelConstraints::new(
            Size::new(160.0, 100.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let (_, panel_node) = builder.panel(key("float-too-tall"), tight);
        let group = builder.tabs(vec![panel_node], 0, false);
        builder.floating_group(
            group,
            Rect::new(Point::new(30.0, 20.0), Size::new(200.0, 90.0)),
            1,
        );
        let layout = builder.build();

        assert!(matches!(
            layout.compute_rects(viewport(1000.0, 100.0), &metrics),
            Err(ComputeRectsError::InsufficientSpace { axis: Axis::Y, .. })
        ));
    }

    #[test]
    fn refuses_to_shrink_canvas_below_its_minimum_without_overflow_policy() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        for (side, key_text) in [
            (DockSide::Left, "canvas-left"),
            (DockSide::Right, "canvas-right"),
        ] {
            let (_, panel_node) = builder.panel(key(key_text), constraints());
            let group = builder.tabs(vec![panel_node], 0, false);
            let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
            builder.main_area(side, column);
        }
        let layout = builder.build();

        let result = layout.compute_rects(viewport(800.0, 500.0), &metrics);
        assert!(matches!(
            result,
            Err(ComputeRectsError::InsufficientCanvasWidth {
                available,
                minimum
            }) if available == 240.0 && minimum == metrics.min_canvas_width
        ));
    }

    #[test]
    fn minimum_group_stack_reports_scroll_extent_instead_of_compressing_content() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) = builder.panel(key(&format!("short-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, index == 0));
        }
        let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.2, 0.3, 0.5]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 120.0), &metrics)
            .unwrap();
        let overflow = rects.vertical_overflow.as_slice();
        assert_eq!(overflow.len(), 1);
        assert_eq!(overflow[0].owner, column);
        assert_eq!(overflow[0].viewport, viewport(280.0, 120.0));
        assert_eq!(overflow[0].content_extent.origin, Point::ZERO);
        assert_eq!(overflow[0].content_extent.size, Size::new(280.0, 268.0));
        assert_eq!(rects.groups.len(), 3);
        assert_eq!(
            rects.groups[0].rect.size.height,
            metrics.collapsed_group_height
        );
        assert_eq!(rects.groups[1].rect.size.height, 116.0);
        assert_eq!(rects.groups[2].rect.size.height, 116.0);
        assert!(
            rects
                .panels
                .iter()
                .all(|panel| panel.rect.size.height >= constraints().min_size.height)
        );
    }

    fn assert_snapshot_rectangles_are_valid(rects: &LayoutRects) {
        crate::layout_rects::assert_layout_rects_invariants(rects);
    }

    #[test]
    fn empty_layout_at_minimum_canvas_width_returns_full_valid_canvas() {
        let metrics = DockMetrics::default();
        let layout = LayoutBuilder::new(Profile::Photoshop).build();
        let window = Rect::new(
            Point::new(5.0, 9.0),
            Size::new(metrics.min_canvas_width, 1.0),
        );

        let rects = layout.compute_rects(window, &metrics).unwrap();

        assert_eq!(rects.canvas, Some(window));
        assert!(rects.columns.is_empty());
        assert!(rects.groups.is_empty());
        assert!(rects.tab_headers.is_empty());
        assert!(rects.tab_overflows.is_empty());
        assert!(rects.panels.is_empty());
        assert!(rects.floating_groups.is_empty());
        assert!(rects.vertical_overflow.is_empty());
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn narrow_window_fails_at_canvas_minimum_instead_of_emitting_negative_rectangles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("narrow-canvas-boundary"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        // 280 px of dock plus the 319 px canvas leaves the Canvas one pixel
        // below its declared minimum of 320 logical px.
        assert!(matches!(
            layout.compute_rects(viewport(599.0, 120.0), &metrics),
            Err(ComputeRectsError::InsufficientCanvasWidth {
                available,
                minimum,
            }) if available == 319.0 && minimum == metrics.min_canvas_width
        ));
        // A window narrower than the dock must fail before constructing a
        // Canvas with negative width.
        assert!(matches!(
            layout.compute_rects(viewport(200.0, 120.0), &metrics),
            Err(ComputeRectsError::InsufficientCanvasWidth {
                available,
                minimum,
            }) if available == -80.0 && minimum == metrics.min_canvas_width
        ));

        let rects = layout
            .compute_rects(viewport(600.0, 120.0), &metrics)
            .unwrap();
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(280.0, 0.0), Size::new(320.0, 120.0)))
        );
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn very_short_column_preserves_panel_minimum_and_reports_full_overflow_extent() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("short-window-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let short_window = viewport(1000.0, 1.0);

        let rects = layout.compute_rects(short_window, &metrics).unwrap();

        let minimum_group_height =
            metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;
        assert_eq!(rects.vertical_overflow.len(), 1);
        assert_eq!(rects.vertical_overflow[0].owner, column);
        assert_eq!(rects.vertical_overflow[0].viewport.size.height, 1.0);
        assert_eq!(
            rects.vertical_overflow[0].content_extent.size.height,
            minimum_group_height
        );
        assert_eq!(rects.groups[0].rect.size.height, minimum_group_height);
        assert_eq!(
            rects.panels[0].rect.size.height,
            constraints().min_size.height
        );
        assert!(rects.groups[0].rect.size.height >= metrics.min_group_height);
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn short_icon_rail_reports_full_scroll_extent_without_compressing_slots() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) = builder.panel(key(&format!("short-rail-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, false));
        }
        let column = builder.column(
            metrics.rail_width,
            ColumnMode::IconsOnly,
            groups.clone(),
            vec![1.0 / 3.0; 3],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout
            .compute_rects(viewport(1000.0, 20.0), &metrics)
            .unwrap();
        let slot_height = metrics.effective_collapsed_group_height();

        assert_eq!(rects.vertical_overflow.len(), 1);
        assert_eq!(rects.vertical_overflow[0].owner, column);
        assert_eq!(
            rects.vertical_overflow[0].content_extent.size.height,
            slot_height * 3.0
        );
        assert_eq!(rects.tab_headers.len(), 3);
        assert!(
            rects
                .tab_headers
                .iter()
                .all(|tab| tab.rect.size.height == slot_height)
        );
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn zero_sized_viewport_is_rejected_before_any_geometry_is_produced() {
        let metrics = DockMetrics::default();
        let layout = LayoutBuilder::new(Profile::Photoshop).build();

        for window in [
            Rect::new(Point::ZERO, Size::new(0.0, 100.0)),
            Rect::new(Point::ZERO, Size::new(100.0, 0.0)),
            Rect::new(Point::ZERO, Size::ZERO),
        ] {
            assert!(matches!(
                layout.compute_rects(window, &metrics),
                Err(ComputeRectsError::Geometry(_))
            ));
        }
    }

    #[test]
    fn collapsed_group_unit_weight_does_not_leave_expanded_column_weight_sum_zero() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, expanded_panel) = builder.panel(key("zero-weight-expanded"), constraints());
        let (_, collapsed_panel) = builder.panel(key("unit-weight-collapsed"), constraints());
        let expanded = builder.tabs(vec![expanded_panel], 0, false);
        let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![expanded, collapsed],
            vec![0.0, 1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));

        // The collapsed group's unit ratio is a fixed strip, not a weight
        // available to the expanded group. Its zero ratio must trigger fallback.
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .expect("valid ratios with a zero expanded-weight sum must lay out");
        let height = |group| {
            rects
                .groups
                .iter()
                .find(|item| item.group == group)
                .unwrap()
                .rect
                .size
                .height
        };
        assert!((height(expanded) - 468.0).abs() < PIXEL_TOLERANCE);
        assert_eq!(height(collapsed), metrics.effective_collapsed_group_height());
        assert!(rects.vertical_overflow.is_empty());
    }

    #[test]
    fn nested_vertical_split_shares_space_when_all_expanded_child_weights_are_zero() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel_a) = builder.panel(key("nested-zero-a"), constraints());
        let (_, panel_b) = builder.panel(key("nested-zero-b"), constraints());
        let (_, collapsed_panel) = builder.panel(key("nested-zero-collapsed"), constraints());
        let group_a = builder.tabs(vec![panel_a], 0, false);
        let group_b = builder.tabs(vec![panel_b], 0, false);
        let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
        let vertical_split = builder.split(
            Axis::Y,
            vec![group_a, group_b, collapsed],
            vec![0.0, 0.0, 1.0],
        );
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![vertical_split],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));

        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .expect("nested Split(Y) must fall back when expanded weights sum to zero");
        let height = |group| {
            rects
                .groups
                .iter()
                .find(|item| item.group == group)
                .unwrap()
                .rect
                .size
                .height
        };
        assert!((height(group_a) - 232.0).abs() < PIXEL_TOLERANCE);
        assert!((height(group_b) - 232.0).abs() < PIXEL_TOLERANCE);
        assert_eq!(height(collapsed), metrics.effective_collapsed_group_height());
        assert!(rects.vertical_overflow.is_empty());
    }

    #[test]
    fn zero_weight_child_keeps_its_minimum_without_changing_positive_weight_proportions() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_zero) = builder.panel(key("mixed-weights-zero"), constraints());
        let (_, panel_quarter) = builder.panel(key("mixed-weights-quarter"), constraints());
        let (_, panel_three_quarters) =
            builder.panel(key("mixed-weights-three-quarters"), constraints());
        let group_zero = builder.tabs(vec![panel_zero], 0, false);
        let group_quarter = builder.tabs(vec![panel_quarter], 0, false);
        let group_three_quarters = builder.tabs(vec![panel_three_quarters], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![group_zero, group_quarter, group_three_quarters],
            vec![0.0, 0.25, 0.75],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        // ratio_issues must accept zero entries when the full vector still sums to one.
        assert_eq!(layout.validate(&metrics), Ok(()));

        let rects = layout
            .compute_rects(viewport(1000.0, 1000.0), &metrics)
            .expect("zero-weight children should be clamped without invalidating other weights");
        let height = |group| {
            rects
                .groups
                .iter()
                .find(|item| item.group == group)
                .unwrap()
                .rect
                .size
                .height
        };
        let zero_height = height(group_zero);
        let quarter_height = height(group_quarter);
        let three_quarters_height = height(group_three_quarters);

        let minimum = metrics
            .header_height
            + metrics.tab_strip_height
            + constraints().min_size.height;
        assert!((zero_height - minimum).abs() < PIXEL_TOLERANCE);
        assert!((quarter_height - 219.0).abs() < PIXEL_TOLERANCE);
        assert!((three_quarters_height - 657.0).abs() < PIXEL_TOLERANCE);
        assert!(
            (quarter_height / three_quarters_height - 0.25 / 0.75).abs()
                < PIXEL_TOLERANCE
        );
        assert!(rects.vertical_overflow.is_empty());
    }

    #[test]
    fn dominant_ratio_keeps_zero_weight_groups_at_minimum_and_gives_remainder_to_dominant_group() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) =
                builder.panel(key(&format!("dominant-ratio-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, false));
        }
        let zero_a = groups[0];
        let zero_b = groups[1];
        let dominant = groups[2];
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            groups,
            vec![0.0, 0.0, 1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));
        let viewport_height = 1000.0;
        let rects = layout
            .compute_rects(viewport(1000.0, viewport_height), &metrics)
            .expect("a dominant positive ratio should absorb the remaining height");

        let height = |wanted| {
            rects
                .groups
                .iter()
                .find(|group| group.group == wanted)
                .expect("every expanded group should emit geometry")
                .rect
                .size
                .height
        };
        let minimum = metrics.header_height
            + metrics.tab_strip_height
            + constraints().min_size.height;

        assert!((height(zero_a) - minimum).abs() < PIXEL_TOLERANCE);
        assert!((height(zero_b) - minimum).abs() < PIXEL_TOLERANCE);
        assert!(
            (height(dominant)
                - (viewport_height - 2.0 * metrics.splitter_thickness - 2.0 * minimum))
                .abs()
                < PIXEL_TOLERANCE
        );
        assert!(height(dominant) > height(zero_a) * 2.0);
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn all_collapsed_groups_keep_their_fixed_strip_height_and_do_not_overflow() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) =
                builder.panel(key(&format!("all-collapsed-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, true));
        }
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            groups.clone(),
            vec![0.0, 1.0, 0.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));
        let rects = layout
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .expect("all-collapsed columns should be valid with a generous viewport");

        assert_eq!(rects.groups.len(), 3);
        assert!(rects.panels.is_empty());
        assert!(rects.vertical_overflow.is_empty());
        for group in groups {
            let rendered = rects
                .groups
                .iter()
                .find(|rendered| rendered.group == group)
                .expect("collapsed group should still emit its rail/header geometry");
            assert_eq!(
                rendered.rect.size.height,
                metrics.effective_collapsed_group_height()
            );
        }
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn ratio_allocation_remains_valid_through_multiple_nested_vertical_splits() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel_a) = builder.panel(key("multi-level-a"), constraints());
        let (_, panel_b) = builder.panel(key("multi-level-b"), constraints());
        let (_, panel_c) = builder.panel(key("multi-level-c"), constraints());
        let (_, panel_d) = builder.panel(key("multi-level-d"), constraints());
        let group_a = builder.tabs(vec![panel_a], 0, false);
        let group_b = builder.tabs(vec![panel_b], 0, false);
        let group_c = builder.tabs(vec![panel_c], 0, false);
        let group_d = builder.tabs(vec![panel_d], 0, false);
        let inner = builder.split(Axis::Y, vec![group_b, group_c], vec![0.0, 1.0]);
        let outer = builder.split(
            Axis::Y,
            vec![group_a, inner, group_d],
            vec![0.25, 0.5, 0.25],
        );
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![outer],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));
        let viewport_height = 1400.0;
        let rects = layout
            .compute_rects(viewport(1000.0, viewport_height), &metrics)
            .expect("nested valid ratios should allocate geometry through every split level");

        let height = |wanted| {
            rects
                .groups
                .iter()
                .find(|group| group.group == wanted)
                .expect("nested tab group should emit geometry")
                .rect
                .size
                .height
        };
        let minimum = metrics.header_height
            + metrics.tab_strip_height
            + constraints().min_size.height;
        let outer_left = height(group_a);
        let outer_right = height(group_d);
        let inner_zero_weight = height(group_b);
        let inner_dominant = height(group_c);

        assert!((outer_left - outer_right).abs() < PIXEL_TOLERANCE);
        assert!((inner_zero_weight - minimum).abs() < PIXEL_TOLERANCE);
        assert!(inner_dominant > minimum);
        assert!(
            ((inner_zero_weight + inner_dominant + metrics.splitter_thickness)
                - 2.0 * outer_left)
                .abs()
                < PIXEL_TOLERANCE
        );
        assert!(rects.vertical_overflow.is_empty());
        assert_snapshot_rectangles_are_valid(&rects);
    }

    #[test]
    fn compute_rects_rejects_child_rect_with_unrepresentable_edge() {
        let metrics = DockMetrics::default();
        let origin_x = 4_294_967_296.0;
        let viewport = Rect::new(Point::new(origin_x, 0.0), Size::new(4096.0, 800.0));
        assert_eq!(viewport.validate_non_empty(), Ok(()));

        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel_a) = builder.panel(key("large-origin-a"), constraints());
        let (_, panel_b) = builder.panel(key("large-origin-b"), constraints());
        let group_a = builder.tabs(vec![panel_a], 0, false);
        let group_b = builder.tabs(vec![panel_b], 0, false);
        // The first child gets the 200px minimum column width. At this
        // coordinate magnitude, f32 spacing is 512px, so that width rounds
        // back to the origin and must not produce a zero-width child.
        let split = builder.split(Axis::X, vec![group_a, group_b], vec![0.01, 0.99]);
        builder.main_area(DockSide::Left, split);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));
        assert!(matches!(
            layout.compute_rects(viewport, &metrics),
            Err(ComputeRectsError::Geometry(
                GeometryError::NonPositiveDimension
            ))
        ));
    }

    #[test]
    fn floating_bounds_clamp_rejects_positive_but_unrepresentable_edge() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("large-origin-floating"), constraints());
        let root = builder.tabs(vec![panel], 0, false);
        let layout = builder.build();
        let bounds = Rect::new(Point::new(2_147_483_648.0, 0.0), Size::new(80.0, 300.0));

        assert_eq!(
            super::clamp_floating_bounds(
                &layout,
                root,
                bounds,
                viewport(800.0, 600.0),
                &metrics,
            ),
            Err(ComputeRectsError::Geometry(
                GeometryError::NonPositiveDimension
            ))
        );
    }

    fn add_perf_tabs_group(
        builder: &mut LayoutBuilder,
        prefix: &str,
        panel_count: usize,
    ) -> crate::NodeId {
        let mut panels = Vec::with_capacity(panel_count);
        for panel_index in 0..panel_count {
            let panel_key = format!("{prefix}-p{panel_index}");
            let (_, panel) = builder.panel(key(&panel_key), constraints());
            panels.push(panel);
        }
        builder.tabs(panels, 0, false)
    }

    fn perf_wide_layout(
        metrics: &DockMetrics,
    ) -> (crate::Layout, Rect, &'static str) {
        let column_count = 10;
        let groups_per_column = 8;
        let panels_per_group = 3;
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut columns = Vec::with_capacity(column_count);

        for column_index in 0..column_count {
            let mut groups = Vec::with_capacity(groups_per_column);
            for group_index in 0..groups_per_column {
                groups.push(add_perf_tabs_group(
                    &mut builder,
                    &format!("rects-wide-c{column_index}-g{group_index}"),
                    panels_per_group,
                ));
            }
            columns.push(builder.column(
                metrics.default_column_width,
                ColumnMode::Expanded,
                groups,
                vec![1.0 / groups_per_column as f32; groups_per_column],
            ));
        }

        let root = builder.split(
            Axis::X,
            columns,
            vec![1.0 / column_count as f32; column_count],
        );
        builder.main_area(DockSide::Left, root);
        let viewport_width = metrics.default_column_width * column_count as f32
            + metrics.splitter_thickness * (column_count - 1) as f32
            + metrics.min_canvas_width
            + 512.0;
        let viewport_height = groups_per_column as f32
            * (metrics.min_group_height + metrics.splitter_thickness)
            + 800.0;
        (
            builder.build(),
            viewport(viewport_width, viewport_height),
            "wide-10x8x3",
        )
    }

    fn perf_deep_layout(
        metrics: &DockMetrics,
    ) -> (crate::Layout, Rect, &'static str) {
        let depth = 24;
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut root = add_perf_tabs_group(&mut builder, "rects-deep-leaf", 1);

        for level in 0..depth {
            let sibling = add_perf_tabs_group(
                &mut builder,
                &format!("rects-deep-level{level}"),
                1,
            );
            // Keep this fixture focused on recursion depth. The wide fixture
            // already exercises horizontal allocation; alternating X/Y splits
            // here can generate a minimum-width stress case that is unrelated
            // to repeated minimum-size traversal.
            root = builder.split(Axis::Y, vec![sibling, root], vec![0.5, 0.5]);
        }
        builder.main_area(DockSide::Left, root);

        let viewport_width = metrics.default_column_width
            + metrics.min_canvas_width
            + 800.0;
        let viewport_height = depth as f32
            * (metrics.min_group_height + metrics.splitter_thickness)
            + 1200.0;
        (
            builder.build(),
            viewport(viewport_width, viewport_height),
            "deep-vertical-24",
        )
    }

    fn perf_many_tabs_layout(
        metrics: &DockMetrics,
    ) -> (crate::Layout, Rect, &'static str) {
        let tabs_count = 128;
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let group = add_perf_tabs_group(&mut builder, "rects-tabs", tabs_count);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);

        let viewport_width = metrics.default_column_width
            + metrics.min_canvas_width
            + 512.0;
        let viewport_height = metrics.min_group_height + 1200.0;
        (
            builder.build(),
            viewport(viewport_width, viewport_height),
            "tabs-128",
        )
    }

    fn visit_summary(
        visits: &std::collections::HashMap<crate::NodeId, super::visit_probe::NodeVisitCounts>,
        field: fn(&super::visit_probe::NodeVisitCounts) -> u64,
    ) -> (u64, usize, u64) {
        let total = visits.values().map(field).sum();
        let repeated_nodes = visits.values().filter(|counts| field(counts) > 1).count();
        let maximum = visits.values().map(field).max().unwrap_or(0);
        (total, repeated_nodes, maximum)
    }

    fn measure_compute_rects_case(
        layout: &crate::Layout,
        viewport: Rect,
        metrics: &DockMetrics,
        label: &str,
        iterations: usize,
    ) {
        use std::{hint::black_box, time::Instant};

        assert_eq!(layout.validate(metrics), Ok(()));
        let initial = layout
            .compute_rects(viewport, metrics)
            .expect("benchmark fixture must compute geometry");
        assert!(!initial.panels.is_empty(), "benchmark should lay out panels");

        for _ in 0..iterations.min(8) {
            let rects = layout
                .compute_rects(viewport, metrics)
                .expect("benchmark fixture must compute geometry");
            black_box((rects.panels.len(), rects.groups.len(), rects.columns.len()));
        }

        let started = Instant::now();
        let mut panel_rects = 0;
        let mut group_rects = 0;
        for _ in 0..iterations {
            let rects = layout
                .compute_rects(viewport, metrics)
                .expect("benchmark fixture must compute geometry");
            panel_rects = rects.panels.len();
            group_rects = rects.groups.len();
            black_box((rects.panels.len(), rects.groups.len(), rects.columns.len()));
        }
        let elapsed = started.elapsed();
        assert!(panel_rects > 0);
        assert!(group_rects > 0);

        // A separate profiled call gathers per-node counts without timing the
        // enabled counter/locking overhead in the benchmark's elapsed duration.
        super::visit_probe::start();
        let profiled = layout.compute_rects(viewport, metrics);
        let visits = super::visit_probe::finish();
        let profiled = profiled.expect("profiled benchmark fixture must compute geometry");
        assert_eq!(profiled.panels.len(), panel_rects);
        assert_eq!(profiled.groups.len(), group_rects);

        let preferred = visit_summary(&visits, |counts| counts.preferred_width);
        let minimum_helper = visit_summary(&visits, |counts| counts.minimum_node_size_helper);
        let minimum_recursive = visit_summary(&visits, |counts| counts.recursive_minimum_node_size);
        let allocation_child = visit_summary(&visits, |counts| counts.allocate_stack_child);
        let total_node_visits: u64 = visits.values().map(|counts| {
            counts.preferred_width
                + counts.minimum_node_size_helper
                + counts.recursive_minimum_node_size
                + counts.allocate_stack_child
        }).sum();
        let unique_nodes = visits.len();

        println!(
            "compute-rects baseline [{}]: nodes={}, panels={}, iterations={}, elapsed_ms={:.3}, us/call={:.3}, calls/s={:.1}, output_panels={}, output_groups={}, unique_instrumented_nodes={}, total_node_visits={}",
            label,
            layout.nodes().count(),
            layout.panel_instances().count(),
            iterations,
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64,
            iterations as f64 / elapsed.as_secs_f64(),
            panel_rects,
            group_rects,
            unique_nodes,
            total_node_visits,
        );
        println!(
            "compute-rects visits [{}]: preferred_width=total/{}/nodes_repeated/{}/max_per_node/{}, minimum_node_size_helper=total/{}/nodes_repeated/{}/max_per_node/{}, recursive_minimum_node_size=total/{}/nodes_repeated/{}/max_per_node/{}, allocate_stack_children=total/{}/nodes_repeated/{}/max_per_node/{}",
            label,
            preferred.0, preferred.1, preferred.2,
            minimum_helper.0, minimum_helper.1, minimum_helper.2,
            minimum_recursive.0, minimum_recursive.1, minimum_recursive.2,
            allocation_child.0, allocation_child.1, allocation_child.2,
        );
    }

    /// Manual release-mode baseline for node-size recomputation during geometry.
    ///
    /// Run with one test thread so that per-node visit counters stay isolated:
    /// cargo test -p dock-core --release benchmark_compute_rects_node_size_visits -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore = "manual performance baseline; run the documented release-mode command"]
    fn benchmark_compute_rects_node_size_visits() {
        let metrics = DockMetrics::default();
        let cases = [
            perf_wide_layout(&metrics),
            perf_deep_layout(&metrics),
            perf_many_tabs_layout(&metrics),
        ];
        for (layout, viewport, label) in cases {
            let node_count = layout.nodes().count();
            let iterations = if node_count > 300 { 20 } else { 60 };
            measure_compute_rects_case(
                &layout,
                viewport,
                &metrics,
                label,
                iterations,
            );
        }
    }

    #[test]
    fn minimum_size_cache_is_scoped_to_each_metrics_pass() {
        let metrics = DockMetrics::default();
        let taller_metrics = DockMetrics {
            header_height: 70.0,
            tab_strip_height: 70.0,
            ..metrics
        };
        assert_eq!(taller_metrics.validate(), Ok(()));

        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("minimum-cache-metrics"), constraints());
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let small_viewport = viewport(1000.0, 150.0);

        let default_rects = layout.compute_rects(small_viewport, &metrics).unwrap();
        let taller_rects = layout
            .compute_rects(small_viewport, &taller_metrics)
            .unwrap();
        let group_height = |rects: &LayoutRects| {
            rects
                .groups
                .iter()
                .find(|rendered| rendered.group == group)
                .unwrap()
                .rect
                .size
                .height
        };

        assert_eq!(group_height(&default_rects), 150.0);
        assert_eq!(group_height(&taller_rects), 200.0);
        assert!(default_rects.vertical_overflow.is_empty());
        assert_eq!(taller_rects.vertical_overflow.len(), 1);
    }

    #[test]
    fn minimum_size_cache_does_not_leak_across_column_modes() {
        let metrics = DockMetrics::default();
        let build_layout = |mode: ColumnMode| {
            let mut builder = LayoutBuilder::new(Profile::Photoshop);
            let (_, panel) =
                builder.panel(key("minimum-cache-column-mode"), constraints());
            let group = builder.tabs(vec![panel], 0, false);
            let width = match mode {
                ColumnMode::Expanded => metrics.default_column_width,
                ColumnMode::IconsAndLabels => metrics.min_column_width,
                ColumnMode::IconsOnly => metrics.rail_width,
            };
            let column = builder.column(width, mode, vec![group], vec![1.0]);
            builder.main_area(DockSide::Left, column);
            let layout = builder.build();
            assert_eq!(layout.validate(&metrics), Ok(()));
            (layout, column)
        };

        let (expanded, _) = build_layout(ColumnMode::Expanded);
        let (icons_and_labels, _) = build_layout(ColumnMode::IconsAndLabels);
        let (icons_only, _) = build_layout(ColumnMode::IconsOnly);

        let expanded_rects = expanded
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let labels_rects = icons_and_labels
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let icons_rects = icons_only
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();

        assert_eq!(
            expanded_rects.columns[0].rect.size.width,
            metrics.default_column_width
        );
        assert_eq!(
            labels_rects.columns[0].rect.size.width,
            metrics.min_column_width
        );
        assert_eq!(
            icons_rects.columns[0].rect.size.width,
            metrics.rail_width
        );
        assert_eq!(expanded_rects.panels.len(), 1);
        assert!(labels_rects.panels.is_empty());
        assert!(icons_rects.panels.is_empty());
    }

    #[test]
    fn minimum_size_cache_does_not_leak_across_collapsed_states() {
        let metrics = DockMetrics::default();
        let build_layout = |collapsed: bool| {
            let mut builder = LayoutBuilder::new(Profile::Generic);
            let (_, panel) =
                builder.panel(key("minimum-cache-collapsed"), constraints());
            let group = builder.tabs(vec![panel], 0, collapsed);
            let column = builder.column(
                metrics.default_column_width,
                ColumnMode::Expanded,
                vec![group],
                vec![1.0],
            );
            builder.main_area(DockSide::Left, column);
            (builder.build(), group)
        };

        let (expanded, expanded_group) = build_layout(false);
        let (collapsed, collapsed_group) = build_layout(true);
        let expanded_rects = expanded
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let collapsed_rects = collapsed
            .compute_rects(viewport(1000.0, 500.0), &metrics)
            .unwrap();
        let rendered_group_height = |rects: &LayoutRects, group| {
            rects
                .groups
                .iter()
                .find(|rendered| rendered.group == group)
                .unwrap()
                .rect
                .size
                .height
        };

        assert_eq!(
            rendered_group_height(&expanded_rects, expanded_group),
            500.0
        );
        assert_eq!(
            rendered_group_height(&collapsed_rects, collapsed_group),
            metrics.effective_collapsed_group_height()
        );
        assert_eq!(expanded_rects.panels.len(), 1);
        assert!(collapsed_rects.panels.is_empty());
    }
}
