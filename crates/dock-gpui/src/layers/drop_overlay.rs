use std::collections::HashMap;

use dock_core::{NodeId, Point, Rect, Size};
use gpui::{AnyElement, IntoElement, Styled, rgb, ScrollHandle};

use crate::components::DropZone;
use crate::geometry::scroll_rect_in_viewport;
use crate::requests::DragOverlay;
use crate::surface::RenderData;
use crate::tree_query::drop_target_node;

use super::{main_areas::ScrollOverflow, rect_element};

pub(crate) fn render(
    data: &RenderData<'_>,
    overlay: Option<&DragOverlay>,
    overflows: &[ScrollOverflow],
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
) -> Vec<AnyElement> {
    let Some(overlay) = overlay else {
        return Vec::new();
    };
    let theme = data.theme;
    let mut elements = Vec::new();

    for zone in &overlay.zones {
        let mut zone_rect = zone.rect;
        if let Some(target_node) = drop_target_node(zone.target)
            && let Some(overflow) = overflows.iter().find(|entry| entry.nodes.contains(&target_node))
        {
            let scroll_offset_y = scroll_handles
                .get(&overflow.owner)
                .map_or(0.0, |handle| handle.offset().y.as_f32());
            let Some(scrolled_rect) =
                scroll_rect_in_viewport(zone.rect, overflow.viewport, scroll_offset_y)
            else {
                continue;
            };
            zone_rect = scrolled_rect;
        }

        elements.push(
            DropZone {
                rect: zone_rect,
                active: overlay.preview.target == Some(zone.target),
                theme: *theme,
            }
            .into_any_element(),
        );
    }

    let marker = Rect::new(
        Point::new(
            (overlay.preview.point.x - 5.0).max(0.0),
            (overlay.preview.point.y - 5.0).max(0.0),
        ),
        Size::new(10.0, 10.0),
    );
    elements.push(
        rect_element(marker, theme.colors.focus)
            .border_1()
            .border_color(rgb(theme.colors.primary_text))
            .into_any_element(),
    );
    elements
}
