use std::collections::HashMap;

#[path = "compute_drop_zones/drop_zone_tree.rs"]
mod drop_zone_tree;
pub(super) use drop_zone_tree::{
    DropZoneTreeIndex, can_add_column, collect_panels, contains_node, direct_parent,
    edge_group_for_column, edge_group_for_dock_edge, group_edge_is_noop, group_main_column,
    is_rail_group, main_columns, owner_column, panel_node, source_allows, source_is_floating_root,
    source_root, target_area,
};

#[path = "compute_drop_zones/drop_zone_floating.rs"]
mod drop_zone_floating;
#[path = "compute_drop_zones/drop_zone_main.rs"]
mod drop_zone_main;

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
    may_add_left_column: bool,
    may_add_right_column: bool,
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

    /// Prepares an owned resolver snapshot for repeated pointer updates in one drag.
    ///
    /// Legal zones, the tree index, and floating-area lookup data are captured once.
    /// The resolver does not borrow this layout, geometry, or metrics, so an adapter
    /// can safely keep it in drag-session state across UI callbacks. The adapter must
    /// discard it before resolving against a changed layout, geometry, or metrics.
    pub fn prepare_drop_resolver(
        &self,
        source: DropSource,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<DropZoneResolver, ComputeRectsError> {
        self.validate_drop_zone_state(metrics)?;
        let tree_index = DropZoneTreeIndex::new(self);
        let zones = self.compute_drop_zones_with_index(source, rects, metrics, &tree_index)?;
        let floating_areas = rects
            .floating_groups
            .iter()
            .filter_map(|floating_rect| {
                let floating = self.floating_group(floating_rect.group)?;
                let area = self.dock_area(floating.area)?;
                Some(FloatingDropArea {
                    rect: floating_rect.rect,
                    z_order: floating_rect.z_order,
                    root: area.root,
                })
            })
            .collect();
        Ok(DropZoneResolver {
            floating_areas,
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
        let viewport = rects
            .viewport
            .ok_or(ComputeRectsError::InvalidLayoutState)?;
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
        let may_add_left_column = can_add_column(
            self,
            tree_index,
            source,
            source_root,
            DockSide::Left,
            metrics,
        );
        let may_add_right_column = can_add_column(
            self,
            tree_index,
            source,
            source_root,
            DockSide::Right,
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
            may_add_left_column,
            may_add_right_column,
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
    result.validate().map_err(ComputeRectsError::Geometry)?;
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

impl Layout {
    fn append_canvas_edge_zones(
        &self,
        context: &DropZoneBuildContext<'_>,
        zones: &mut Vec<DropZoneRect>,
    ) -> Result<(), ComputeRectsError> {
        let DropZoneBuildContext {
            source,
            source_root,
            source_panels,
            tree_index,
            rects,
            metrics,
            viewport,
            left_columns,
            right_columns,
            may_add_left_column,
            may_add_right_column,
        } = *context;
        // The two outside viewport edges open/extend a dock. A whole group uses
        // its edge operation when a neighbouring column exists; panels use an exact slot.
        if source_allows(self, source_panels, AllowedArea::MAIN_DOCK) {
            let edge_width = metrics.edge_drop_width.min(viewport.size.width / 3.0);
            for (side, columns, zone, is_left) in [
                (DockSide::Left, left_columns, DropZone::DockEdge, true),
                (DockSide::Right, right_columns, DropZone::DockEdge, false),
            ] {
                let may_add_column = match side {
                    DockSide::Left => may_add_left_column,
                    DockSide::Right => may_add_right_column,
                };
                if !may_add_column {
                    continue;
                }
                let rect = if is_left {
                    Rect::new(viewport.origin, Size::new(edge_width, viewport.size.height))
                } else {
                    Rect::new(
                        Point::new(
                            viewport.origin.x + viewport.size.width - edge_width,
                            viewport.origin.y,
                        ),
                        Size::new(edge_width, viewport.size.height),
                    )
                };
                let target = match source {
                    DropSource::Panel { .. } => Some(DropTarget::MainColumn {
                        side,
                        index: if is_left { 0 } else { columns.len() },
                    }),
                    DropSource::Group { .. } => edge_group_for_dock_edge(self, columns, is_left)
                        .map(|group| DropTarget::GroupEdge {
                            group,
                            zone: if is_left {
                                DropZone::Left
                            } else {
                                DropZone::Right
                            },
                        }),
                };
                if let Some(target) = target {
                    let is_noop = match target {
                        DropTarget::GroupEdge { group, zone } => {
                            group_edge_is_noop(self, tree_index, source, group, zone)
                        }
                        _ => false,
                    };
                    if !is_noop {
                        push_zone(zones, target, zone, rect)?;
                    }
                }
            }
        }

        // A free Canvas is a Float command hint, never a fake tab/column destination.
        if source_allows(self, source_panels, AllowedArea::FLOATING)
            && !source_is_floating_root(self, source_root)
            && let Some(canvas) = rects.canvas
        {
            push_zone(zones, DropTarget::FreeArea, DropZone::FreeArea, canvas)?;
        }
        Ok(())
    }
}

/// Reusable drop-resolution snapshot for a single drag gesture.
///
/// Preparing a resolver validates the source layout and geometry, then owns the tree
/// index, legal zones, and floating-area roots needed for subsequent pointer hit tests.
/// It intentionally does not borrow the input data, so callers can keep it in persistent
/// UI state. Callers must discard/rebuild the snapshot after layout, geometry, or metrics
/// change; ownership prevents dangling references, but does not make stale geometry current.
pub struct DropZoneResolver {
    floating_areas: Vec<FloatingDropArea>,
    tree_index: DropZoneTreeIndex,
    zones: Vec<DropZoneRect>,
}

#[derive(Clone, Copy)]
struct FloatingDropArea {
    rect: Rect,
    z_order: i32,
    root: NodeId,
}

impl DropZoneResolver {
    /// Returns the legal drop zones captured when this resolver was prepared.
    ///
    /// UI adapters can render the same geometry used for hit resolution without
    /// recomputing the zones for every pointer movement.
    pub fn zones(&self) -> &[DropZoneRect] {
        &self.zones
    }

    /// Resolves a pointer position using the cached legal zones and tree index.
    ///
    /// This performs only point validation and hit resolution; layout validation,
    /// target enumeration, and zone construction were done when the resolver was prepared.
    pub fn resolve(&self, point: Point) -> Result<Option<DropTarget>, ComputeRectsError> {
        Ok(self.resolve_zone(point)?.map(|zone| zone.target))
    }

    /// Returns the exact legal affordance selected by Core for this point.
    ///
    /// Render adapters should use this result rather than independently scanning
    /// `zones()` or duplicating hit priority. The returned value is copied from
    /// the resolver's authoritative zone list.
    pub fn resolve_zone(&self, point: Point) -> Result<Option<DropZoneRect>, ComputeRectsError> {
        point.validate().map_err(ComputeRectsError::Geometry)?;
        self.resolve_zone_validated(point).map(|zone| zone.copied())
    }

    fn resolve_validated(&self, point: Point) -> Result<Option<DropTarget>, ComputeRectsError> {
        Ok(self.resolve_zone_validated(point)?.map(|zone| zone.target))
    }

    fn resolve_zone_validated(
        &self,
        point: Point,
    ) -> Result<Option<&DropZoneRect>, ComputeRectsError> {
        let mut topmost_floating: Option<&FloatingDropArea> = None;
        for floating in &self.floating_areas {
            if floating
                .rect
                .contains(point)
                .map_err(ComputeRectsError::Geometry)?
            {
                let replace = topmost_floating
                    .as_ref()
                    .map(|current| floating.z_order > current.z_order)
                    .unwrap_or(true);
                if replace {
                    topmost_floating = Some(floating);
                }
            }
        }
        if let Some(floating) = topmost_floating {
            return best_containing_zone(&self.zones, point, |zone| {
                matches!(
                    zone.zone,
                    DropZone::FloatingCenter
                        | DropZone::FloatingTop
                        | DropZone::FloatingBottom
                        | DropZone::TabInsertion
                        | DropZone::Left
                        | DropZone::Right
                ) && zone
                    .target
                    .referenced_node_id()
                    .map(|node| contains_node(&self.tree_index, floating.root, node))
                    .unwrap_or(false)
            });
        }
        best_containing_zone(&self.zones, point, |_| true)
    }
}

fn best_containing_zone<'a>(
    zones: &'a [DropZoneRect],
    point: Point,
    include: impl Fn(&DropZoneRect) -> bool,
) -> Result<Option<&'a DropZoneRect>, ComputeRectsError> {
    let mut best: Option<(u8, usize)> = None;
    for (index, zone) in zones.iter().enumerate() {
        if !include(zone)
            || !zone
                .rect
                .contains(point)
                .map_err(ComputeRectsError::Geometry)?
        {
            continue;
        }
        let priority = zone_priority(zone.zone);
        if best
            .map(|(best_priority, _)| priority < best_priority)
            .unwrap_or(true)
        {
            best = Some((priority, index));
        }
    }
    Ok(best.and_then(|(_, index)| zones.get(index)))
}

fn zone_priority(zone: DropZone) -> u8 {
    match zone {
        DropZone::RailInsertion => 0,
        DropZone::TabInsertion => 1,
        DropZone::DockEdge => 2,
        DropZone::ColumnLeft | DropZone::ColumnRight => 3,
        DropZone::ColumnEnd => 4,
        DropZone::FloatingTop | DropZone::FloatingBottom => 5,
        DropZone::Top | DropZone::Bottom => 6,
        DropZone::Left | DropZone::Right => 7,
        DropZone::FloatingCenter => 8,
        DropZone::Center => 9,
        DropZone::FreeArea => 10,
    }
}
