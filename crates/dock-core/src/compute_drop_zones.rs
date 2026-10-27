use std::collections::HashMap;

#[path = "compute_drop_zones/drop_zone_tree.rs"]
mod drop_zone_tree;
pub(super) use drop_zone_tree::{
    can_add_column, collect_panels, contains_node, direct_parent, edge_group_for_column,
    edge_group_for_dock_edge, group_edge_is_noop, group_main_column, is_rail_group,
    main_columns, owner_column,
    panel_node, source_allows, source_is_floating_root, source_root, target_area,
    DropZoneTreeIndex,
};

#[path = "compute_drop_zones/drop_zone_resolver.rs"]
mod drop_zone_resolver;
pub use drop_zone_resolver::DropZoneResolver;

#[path = "compute_drop_zones/drop_zone_main.rs"]
mod drop_zone_main;
#[path = "compute_drop_zones/drop_zone_floating.rs"]
mod drop_zone_floating;
#[path = "compute_drop_zones/drop_zone_canvas.rs"]
mod drop_zone_canvas;

use crate::{
    AllowedArea, ComputeRectsError, DockMetrics, DockSide, DropSource, DropTarget, DropZone,
    DropZoneRect, FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId, Point, Profile,
    Rect, Size,
};

#[derive(Clone, Copy)]
struct DropZoneBuildContext<'a> {
    source: DropSource,
    source_root: NodeId,
    source_panels: &'a [PanelId],
    tree_index: &'a DropZoneTreeIndex,
    rects: &'a LayoutRects,
    metrics: &'a DockMetrics,
    viewport: Rect,
    left_columns: &'a [(NodeId, Rect)],
    right_columns: &'a [(NodeId, Rect)],
    may_add_column: bool,
}

impl Layout {
    /// Builds source-aware drop affordances from an already computed geometry snapshot.
    ///
    /// Only legal targets are emitted. The same snapshot must have been produced by
    /// this Layout; a default/uncomputed snapshot is rejected. Drop zones are separate
    /// from compute_rects because permissions depend on the dragged item.
    pub fn compute_drop_zones(
        &self,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<Vec<DropZoneRect>, ComputeRectsError> {
        self.validate_drop_zone_state(metrics)?;
        let tree_index = DropZoneTreeIndex::new(self);
        self.compute_drop_zones_with_index(source, rects, metrics, &tree_index)
    }

    /// Prepares a reusable drop resolver for repeated pointer updates in one drag.
    ///
    /// The returned value computes legal zones and the tree index once. Its immutable
    /// borrows tie the cache to this layout, geometry snapshot, and metrics; drop it and
    /// prepare a new one after any of those inputs need to change.
    pub fn prepare_drop_resolver<'a>(
        &'a self,
        source: DropSource,
        rects: &'a LayoutRects,
        metrics: &'a DockMetrics,
    ) -> Result<DropZoneResolver<'a>, ComputeRectsError> {
        self.validate_drop_zone_state(metrics)?;
        let tree_index = DropZoneTreeIndex::new(self);
        let zones = self.compute_drop_zones_with_index(source, rects, metrics, &tree_index)?;
        Ok(DropZoneResolver {
            layout: self,
            rects,
            _metrics: metrics,
            tree_index,
            zones,
        })
    }

    fn validate_drop_zone_state(&self, metrics: &DockMetrics) -> Result<(), ComputeRectsError> {
        metrics
            .validate()
            .map_err(|reason| ComputeRectsError::InvalidMetrics { reason })?;
        self.validate_cached(metrics)
            .map_err(|errors| ComputeRectsError::InvalidLayout { errors })?;
        Ok(())
    }

    fn compute_drop_zones_with_index(
        &self,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
        tree_index: &DropZoneTreeIndex,
    ) -> Result<Vec<DropZoneRect>, ComputeRectsError> {
        let viewport = rects.viewport.ok_or(ComputeRectsError::InvalidLayoutState)?;
        viewport
            .validate_non_empty()
            .map_err(ComputeRectsError::Geometry)?;
        let source_root = source_root(self, source, tree_index)?;
        let source_panels = collect_panels(self, source_root);
        if source_panels.is_empty()
            || source_panels
                .iter()
                .any(|panel| self.panel_instance(*panel).is_none())
        {
            return Err(ComputeRectsError::InvalidLayoutState);
        }

        // Validate the geometry entries we are about to use. A stale target ID
        // is ignored per target, while invalid numeric geometry is a typed error.
        for entry in rects.groups.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.columns.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.tab_strips.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.tab_headers.iter() {
            entry.rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
        for entry in rects.floating_groups.iter() {
            entry
                .rect
                .validate_non_empty()
                .map_err(ComputeRectsError::Geometry)?;
        }

        let left_columns = main_columns(self, rects, DockSide::Left);
        let right_columns = main_columns(self, rects, DockSide::Right);
        let may_add_column = can_add_column(
            self,
            tree_index,
            source,
            source_root,
            &left_columns,
            &right_columns,
            metrics,
        );
        let mut zones = Vec::new();

        let context = DropZoneBuildContext {
            source,
            source_root,
            source_panels: &source_panels,
            tree_index,
            rects,
            metrics,
            viewport,
            left_columns: &left_columns,
            right_columns: &right_columns,
            may_add_column,
        };
        self.append_main_group_zones(&context, &mut zones)?;
        self.append_tab_insertion_zones(&context, &mut zones)?;
        self.append_main_column_zones(&context, &mut zones)?;
        self.append_rail_insertion_zones(&context, &mut zones)?;
        self.append_floating_window_zones(&context, &mut zones)?;
        self.append_canvas_edge_zones(&context, &mut zones)?;
        Ok(zones)
    }

    /// Resolves the point to the highest-priority legal destination, if any.
    ///
    /// This one-shot convenience method prepares a temporary resolver. Call
    /// `prepare_drop_resolver` and reuse its result when resolving multiple pointer
    /// positions during one drag gesture.
    pub fn resolve_drop(
        &self,
        point: Point,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<Option<DropTarget>, ComputeRectsError> {
        point.validate().map_err(ComputeRectsError::Geometry)?;
        self.prepare_drop_resolver(source, rects, metrics)?
            .resolve_validated(point)
    }
}

#[cfg(test)]
#[path = "compute_drop_zones/tests.rs"]
mod tests;

pub(super) fn push_zone(
    zones: &mut Vec<DropZoneRect>,
    target: DropTarget,
    zone: DropZone,
    rect: Rect,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return Ok(());
    }
    zones.push(DropZoneRect::new(target, zone, rect));
    Ok(())
}

pub(super) fn group_edge_rect(
    rect: Rect,
    zone: DropZone,
    edge_width: f32,
) -> Result<Rect, ComputeRectsError> {
    let x_band = edge_width.min(rect.size.width / 2.0);
    let y_band = edge_width.min(rect.size.height / 2.0);
    let side_height = (rect.size.height - 2.0 * y_band).max(0.0);
    let result = match zone {
        DropZone::Left => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + y_band),
            Size::new(x_band, side_height),
        ),
        DropZone::Right => Rect::new(
            Point::new(
                rect.origin.x + rect.size.width - x_band,
                rect.origin.y + y_band,
            ),
            Size::new(x_band, side_height),
        ),
        DropZone::Top => Rect::new(rect.origin, Size::new(rect.size.width, y_band)),
        DropZone::Bottom => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height - y_band),
            Size::new(rect.size.width, y_band),
        ),
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn group_center_rect(
    rect: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.edge_drop_width.min(rect.size.width / 3.0);
    let content_top = (metrics.header_height + metrics.tab_strip_height).min(rect.size.height);
    let inset_bottom = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, rect.origin.y + content_top),
        Size::new(
            (rect.size.width - 2.0 * inset_x).max(0.0),
            (rect.size.height - content_top - inset_bottom).max(0.0),
        ),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn floating_edge_rect_for_branch(
    floating_bounds: Rect,
    column_rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let edge_rect = floating_edge_rect(floating_bounds, edge, metrics)?;
    let edge_right = edge_rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let column_right = column_rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let left = edge_rect.origin.x.max(column_rect.origin.x);
    let right = edge_right.min(column_right);
    let result = Rect::new(
        Point::new(left, edge_rect.origin.y),
        Size::new((right - left).max(0.0), edge_rect.size.height),
    );
    result
        .validate()
        .map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn floating_edge_rect(
    rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.splitter_hit_zone.min(rect.size.width / 3.0);
    let height = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let y = match edge {
        // Keep stack insertion below the floating title and tab-strip hit bands.
        DropZone::Top => rect.origin.y + metrics.header_height + metrics.tab_strip_height,
        DropZone::Bottom => rect.origin.y + rect.size.height - height,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, y),
        Size::new((rect.size.width - 2.0 * inset_x).max(0.0), height),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}
