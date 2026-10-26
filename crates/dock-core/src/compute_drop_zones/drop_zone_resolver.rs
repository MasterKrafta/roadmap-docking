use super::*;

/// Reusable drop-resolution snapshot for a single drag gesture.
///
/// Preparing a resolver validates the layout and geometry, then builds the tree index
/// and legal drop-zone list once. Keeping this value alive borrows the layout, geometry,
/// and metrics immutably, preventing the cached snapshot from being used while any of
/// those inputs are changed. Create a new resolver after a layout or viewport update.
pub struct DropZoneResolver<'a> {
    pub(super) layout: &'a Layout,
    pub(super) rects: &'a LayoutRects,
    pub(super) _metrics: &'a DockMetrics,
    pub(super) tree_index: DropZoneTreeIndex,
    pub(super) zones: Vec<DropZoneRect>,
}

impl DropZoneResolver<'_> {
    /// Resolves a pointer position using the cached legal zones and tree index.
    ///
    /// This performs only point validation and hit resolution; layout validation,
    /// target enumeration, and zone construction were done when the resolver was prepared.
    pub fn resolve(&self, point: Point) -> Result<Option<DropTarget>, ComputeRectsError> {
        point.validate().map_err(ComputeRectsError::Geometry)?;
        self.resolve_validated(point)
    }

    pub(super) fn resolve_validated(
        &self,
        point: Point,
    ) -> Result<Option<DropTarget>, ComputeRectsError> {
        let mut topmost_floating: Option<(FloatingGroupId, Rect, i32)> = None;
        for floating in &self.rects.floating_groups {
            if floating
                .rect
                .contains(point)
                .map_err(ComputeRectsError::Geometry)?
            {
                let replace = topmost_floating
                    .as_ref()
                    .map(|(_, _, z)| floating.z_order > *z)
                    .unwrap_or(true);
                if replace {
                    topmost_floating = Some((floating.group, floating.rect, floating.z_order));
                }
            }
        }
        if let Some((floating_id, _bounds, _z_order)) = topmost_floating {
            let Some(floating) = self.layout.floating_group(floating_id) else {
                return Ok(None);
            };
            let Some(area) = self.layout.dock_area(floating.area) else {
                return Ok(None);
            };
            return best_containing_zone(&self.zones, point, |zone| {
                matches!(
                    zone.zone,
                    DropZone::FloatingCenter
                        | DropZone::FloatingTop
                        | DropZone::FloatingBottom
                        | DropZone::TabInsertion
                        | DropZone::Left
                        | DropZone::Right
                ) && zone.target.referenced_node_id()
                    .map(|node| contains_node(&self.tree_index, area.root, node))
                    .unwrap_or(false)
            });
        }
        best_containing_zone(&self.zones, point, |_| true)
    }
}

pub(crate) fn best_containing_zone(
    zones: &[DropZoneRect],
    point: Point,
    include: impl Fn(&DropZoneRect) -> bool,
) -> Result<Option<DropTarget>, ComputeRectsError> {
    let mut best: Option<(u8, usize, DropTarget)> = None;
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
            .map(|(best_priority, _, _)| priority < best_priority)
            .unwrap_or(true)
        {
            best = Some((priority, index, zone.target));
        }
    }
    Ok(best.map(|(_, _, target)| target))
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
