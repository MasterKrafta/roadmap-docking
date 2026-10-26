use std::collections::HashMap;

#[path = "compute_drop_zones/drop_zone_tree.rs"]
mod drop_zone_tree;
pub(super) use drop_zone_tree::{
    can_add_column, collect_main_columns, collect_panels, contains_node, direct_parent,
    edge_group_for_column, edge_group_for_dock_edge, floating_owner, group_edge_is_noop,
    group_main_column, is_rail_group, main_columns, main_side_for_node, owner_column,
    panel_node, source_allows, source_is_floating_root, source_root, target_area,
    DropZoneTreeIndex,
};

#[path = "compute_drop_zones/drop_zone_geometry.rs"]
mod drop_zone_geometry;
pub(super) use drop_zone_geometry::{
    floating_edge_rect, floating_edge_rect_for_branch, group_center_rect, group_edge_rect,
    push_zone,
};

#[path = "compute_drop_zones/drop_zone_resolver.rs"]
mod drop_zone_resolver;
pub use drop_zone_resolver::DropZoneResolver;
pub(super) use drop_zone_resolver::best_containing_zone;

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

        self.append_main_group_zones(
            source,
            source_root,
            &source_panels,
            tree_index,
            rects,
            metrics,
            &left_columns,
            &right_columns,
            may_add_column,
            &mut zones,
        )?;
        self.append_tab_insertion_zones(source, &source_panels, tree_index, rects, &mut zones)?;
        self.append_main_column_zones(
            source,
            &source_panels,
            tree_index,
            rects,
            metrics,
            &left_columns,
            &right_columns,
            may_add_column,
            &mut zones,
        )?;
        self.append_rail_insertion_zones(
            source,
            &source_panels,
            tree_index,
            rects,
            &left_columns,
            &right_columns,
            &mut zones,
        )?;
        self.append_floating_window_zones(
            source,
            source_root,
            &source_panels,
            tree_index,
            rects,
            metrics,
            &mut zones,
        )?;
        self.append_canvas_edge_zones(
            source,
            source_root,
            &source_panels,
            tree_index,
            rects,
            metrics,
            viewport,
            &left_columns,
            &right_columns,
            may_add_column,
            &mut zones,
        )?;
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
