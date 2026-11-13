use std::collections::HashMap;

use dock_core::{
    DockSide, DragPayload, DragPreview, DropTarget, DropZone, DropZoneRect, GroupRect, NodeId,
    Point, Rect, TabHeaderRect, TabStripRect,
};

pub(super) fn composition_edge_highlight_rect(
    zone: DropZoneRect,
    target: DropTarget,
    main_dock_bounds: Option<(&HashMap<DockSide, Rect>, &HashMap<NodeId, DockSide>)>,
) -> Rect {
    let Some((area_bounds, node_sides)) = main_dock_bounds else {
        return zone.rect;
    };
    let side = match target {
        DropTarget::MainColumn { side, .. } => side,
        DropTarget::GroupEdge { group, .. } => {
            let Some(side) = node_sides.get(&group).copied() else {
                return zone.rect;
            };
            side
        }
        _ => return zone.rect,
    };
    let Some(area) = area_bounds.get(&side) else {
        return zone.rect;
    };

    Rect::new(
        Point::new(zone.rect.origin.x, area.origin.y),
        dock_core::Size::new(zone.rect.size.width, area.size.height),
    )
}

pub(super) fn tab_strip_group_highlight_rect(
    target: DropTarget,
    groups: &[GroupRect],
    tab_strips: &[TabStripRect],
) -> Option<Rect> {
    let group = target_group(target)
        .and_then(|id| groups.iter().find(|group| group.group == id))?;
    let strip = tab_strips
        .iter()
        .find(|strip| strip.group == group.group)?;

    // Match the Core TabInsertion hit-area bounds: the visible strip can stop
    // before trailing controls, but the drop target spans the complete row.
    let left = group.rect.origin.x.min(strip.rect.origin.x);
    let right = group.rect.checked_right().ok()?.max(strip.rect.checked_right().ok()?);
    Some(Rect::new(
        Point::new(left, strip.rect.origin.y),
        dock_core::Size::new(right - left, strip.rect.size.height),
    ))
}


pub(super) fn target_group(target: DropTarget) -> Option<dock_core::NodeId> {
    match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::MainColumn { .. }
        | DropTarget::ColumnSlot { .. }
        | DropTarget::RailSlot { .. }
        | DropTarget::FreeArea => None,
    }
}


const DROP_EDGE_GUIDE_THICKNESS: f32 = 4.0;

pub(super) fn centered_vertical_split_guide_rect(rect: Rect) -> Rect {
    let center_x = rect.origin.x + rect.size.width * 0.5;
    Rect::new(
        Point::new(
            center_x - DROP_EDGE_GUIDE_THICKNESS * 0.5,
            rect.origin.y,
        ),
        dock_core::Size::new(DROP_EDGE_GUIDE_THICKNESS, rect.size.height),
    )
}

pub(super) fn floating_side_edge_guide_rect(
    floating_bounds: Rect,
    zone_rect: Rect,
    zone: DropZone,
) -> Option<Rect> {
    let bounds_right = floating_bounds.origin.x + floating_bounds.size.width;
    let zone_right = zone_rect.origin.x + zone_rect.size.width;
    let spans_window_height = (zone_rect.origin.y - floating_bounds.origin.y).abs() <= 1.0
        && (zone_rect.size.height - floating_bounds.size.height).abs() <= 1.0;
    let is_outer_edge = spans_window_height
        && match zone {
            DropZone::Left => (zone_rect.origin.x - floating_bounds.origin.x).abs() <= 1.0,
            DropZone::Right => (zone_right - bounds_right).abs() <= 1.0,
            _ => false,
        };
    if !is_outer_edge {
        return None;
    }

    let thickness = DROP_EDGE_GUIDE_THICKNESS;
    let x = match zone {
        DropZone::Left => floating_bounds.origin.x,
        DropZone::Right => bounds_right - thickness,
        _ => return None,
    };
    Some(Rect::new(
        Point::new(x, floating_bounds.origin.y),
        dock_core::Size::new(thickness, floating_bounds.size.height),
    ))
}

pub(super) fn column_end_visual_rect(zone_rect: Rect, column_rect: Rect) -> Rect {
    Rect::new(
        Point::new(column_rect.origin.x, zone_rect.origin.y),
        dock_core::Size::new(column_rect.size.width, zone_rect.size.height),
    )
}

pub(super) fn column_end_guide_rect(rect: Rect) -> Rect {
    Rect::new(
        Point::new(
            rect.origin.x,
            rect.origin.y + rect.size.height - DROP_EDGE_GUIDE_THICKNESS,
        ),
        dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
    )
}

pub(super) fn group_edge_guide_rect(rect: Rect, zone: DropZone) -> Rect {
    let right = rect.origin.x + rect.size.width;
    let x = match zone {
        DropZone::Left => rect.origin.x,
        DropZone::Right => right - DROP_EDGE_GUIDE_THICKNESS,
        _ => rect.origin.x,
    };
    match zone {
        DropZone::Left | DropZone::Right => Rect::new(
            Point::new(x, rect.origin.y),
            dock_core::Size::new(DROP_EDGE_GUIDE_THICKNESS, rect.size.height),
        ),
        DropZone::Top | DropZone::Bottom => {
            let y = rect.origin.y + rect.size.height * 0.5
                - DROP_EDGE_GUIDE_THICKNESS * 0.5;
            Rect::new(
                Point::new(rect.origin.x, y),
                dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
            )
        }
        _ => rect,
    }
}

pub(super) fn group_edge_visual_rect(rect: Rect, target_group: Option<Rect>) -> Rect {
    let Some(group) = target_group else {
        return rect;
    };

    let left = rect.origin.x.min(group.origin.x);
    let right = (rect.origin.x + rect.size.width)
        .max(group.origin.x + group.size.width);
    Rect::new(
        Point::new(left, rect.origin.y),
        dock_core::Size::new(right - left, rect.size.height),
    )
}

pub(super) fn group_edge_highlight_rect(rect: Rect, zone: DropZone) -> Rect {
    match zone {
        // Keep the full width, but reduce the translucent highlight to half
        // the hit-zone height below the composition header.
        DropZone::Top | DropZone::FloatingTop => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height),
            dock_core::Size::new(rect.size.width, rect.size.height * 0.5),
        ),
        _ => rect,
    }
}

pub(super) fn main_group_edge_guide_rect(
    rect: Rect,
    zone: DropZone,
    target_group: Option<Rect>,
) -> Rect {
    let centered = || group_edge_guide_rect(rect, zone);

    match zone {
        DropZone::Top => {
            let y = rect.origin.y + rect.size.height - DROP_EDGE_GUIDE_THICKNESS * 0.5;
            Rect::new(
                Point::new(rect.origin.x, y),
                dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
            )
        }
        DropZone::Bottom => {
            let zone_bottom = rect.origin.y + rect.size.height;
            let is_last_group = target_group.is_some_and(|group| {
                let group_bottom = group.origin.y + group.size.height;
                (zone_bottom - group_bottom).abs() <= 1.0
            });

            if is_last_group {
                let bottom = target_group
                    .map(|group| group.origin.y + group.size.height)
                    .unwrap_or(zone_bottom);
                Rect::new(
                    Point::new(rect.origin.x, bottom - DROP_EDGE_GUIDE_THICKNESS),
                    dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
                )
            } else {
                centered()
            }
        }
        _ => centered(),
    }
}

pub(super) fn floating_group_edge_guide_rect(
    rect: Rect,
    zone: DropZone,
    floating_bounds: Option<Rect>,
) -> Rect {
    let y = match zone {
        DropZone::FloatingTop => {
            rect.origin.y + rect.size.height - DROP_EDGE_GUIDE_THICKNESS * 0.5
        }
        DropZone::FloatingBottom => floating_bounds
            .map(|bounds| bounds.origin.y + bounds.size.height - DROP_EDGE_GUIDE_THICKNESS)
            .unwrap_or(rect.origin.y + rect.size.height - DROP_EDGE_GUIDE_THICKNESS),
        _ => return group_edge_guide_rect(rect, zone),
    };

    Rect::new(
        Point::new(rect.origin.x, y),
        dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
    )
}

pub(super) fn same_group_panel_tab_insertion_rect(
    preview: DragPreview,
    target: DropTarget,
    headers: &[TabHeaderRect],
    strips: &[TabStripRect],
) -> Option<Rect> {
    let DragPayload::Panel { panel } = preview.payload else {
        return None;
    };
    let DropTarget::Tabs { group, .. } = target else {
        return None;
    };
    let Some(source_header) = headers.iter().find(|header| header.panel == panel) else {
        return None;
    };
    if source_header.group != group {
        return None;
    }
    if !strips.iter().any(|strip| strip.group == group) {
        return None;
    }
    tab_insertion_rect(target, headers, strips)
}

pub(super) fn tab_insertion_rect(
    target: DropTarget,
    headers: &[TabHeaderRect],
    strips: &[TabStripRect],
) -> Option<Rect> {
    let (group, index) = match target {
        DropTarget::Tabs { group, index } | DropTarget::GroupTabs { group, index } => {
            (group, index)
        }
        _ => return None,
    };
    let strip = strips.iter().find(|strip| strip.group == group)?;
    let x = headers
        .iter()
        .find(|header| {
            header.group == group && (header.index == index || header.index + 1 == index)
        })
        .map(|header| {
            if header.index == index {
                header.rect.origin.x
            } else {
                header.rect.origin.x + header.rect.size.width
            }
        })
        .unwrap_or_else(|| {
            if index == 0 {
                strip.rect.origin.x
            } else {
                strip.rect.origin.x + strip.rect.size.width
            }
        });
    Some(Rect::new(
        Point::new(x - DROP_EDGE_GUIDE_THICKNESS * 0.5, strip.rect.origin.y),
        dock_core::Size::new(DROP_EDGE_GUIDE_THICKNESS, strip.rect.size.height),
    ))
}

pub(super) fn rail_insertion_rect(zone: &DropZoneRect, headers: &[TabHeaderRect]) -> Option<Rect> {
    let DropTarget::RailSlot { .. } = zone.target else {
        return None;
    };
    let matching = headers.iter().find(|header| {
        let header_bottom = header.rect.origin.y + header.rect.size.height;
        let zone_bottom = zone.rect.origin.y + zone.rect.size.height;
        header.rect.origin.x == zone.rect.origin.x
            && zone.rect.origin.y >= header.rect.origin.y
            && zone_bottom <= header_bottom
    });
    let y = matching
        .map(|header| {
            let header_middle = header.rect.origin.y + header.rect.size.height * 0.5;
            if zone.rect.origin.y < header_middle {
                header.rect.origin.y
            } else {
                header.rect.origin.y + header.rect.size.height
            }
        })
        .unwrap_or(zone.rect.origin.y + zone.rect.size.height * 0.5);
    Some(Rect::new(
        Point::new(
            zone.rect.origin.x,
            y - DROP_EDGE_GUIDE_THICKNESS * 0.5,
        ),
        dock_core::Size::new(zone.rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
    ))
}

pub(super) fn main_dock_edge_guide_rect(rect: Rect, side: DockSide) -> Rect {
    group_edge_guide_rect(
        rect,
        match side {
            DockSide::Left => DropZone::Left,
            DockSide::Right => DropZone::Right,
        },
    )
}

