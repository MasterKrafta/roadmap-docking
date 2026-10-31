use std::collections::HashMap;

use dock_core::{
    DragPayload, DragPreview, DropTarget, DropZone, DropZoneRect, FloatingGroupId,
    FloatingGroupRect, GroupRect, PanelId, Point, Rect, TabHeaderRect, TabStripRect,
};
use gpui::{
    IntoElement, ParentElement, SharedString, Styled, div, img, px, rgb,
};
use ui::Theme;

use super::tab_strip::{GroupPresentation, TabPresentation};

/// Visual feedback for the current Core-resolved destination.
///
/// The adapter never creates a target here: only zones captured by
/// DropZoneResolver and equal to DragPreview.target are painted.
pub struct DropOverlay;

/// A non-interactive visual card that follows the active drag pointer.
pub struct DragGhost;

impl DropOverlay {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        zones: &[DropZoneRect],
        preview: Option<DragPreview>,
        groups: &[GroupRect],
        tab_headers: &[TabHeaderRect],
        tab_strips: &[TabStripRect],
        floating_groups: &[FloatingGroupRect],
        free_area_bounds: Option<Rect>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut layer = div().absolute().top_0().left_0().size_full();
        let Some(preview) = preview else {
            return layer;
        };
        let Some(target) = preview.target else {
            return layer;
        };

        for zone in zones.iter().filter(|zone| zone.target == target) {
            match zone.zone {
                DropZone::Center => {
                    if let Some(group) = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                    {
                        layer = layer.child(highlight_rect(
                            group.rect,
                            theme,
                            0.12,
                            true,
                        ));
                    }
                }
                DropZone::TabInsertion => {
                    if let Some(line) = tab_insertion_rect(
                        target,
                        zone.rect,
                        tab_headers,
                        tab_strips,
                    ) {
                        layer = layer.child(guide_rect(line, theme));
                    }
                }
                DropZone::Left | DropZone::Right | DropZone::Top | DropZone::Bottom => {
                    if let Some(group) = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                    {
                        let half = group_edge_half(group.rect, zone.zone);
                        layer = layer.child(highlight_rect(half, theme, 0.25, true));
                    }
                }
                DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge => {
                    let x = zone.rect.origin.x + zone.rect.size.width * 0.5;
                    let line = Rect::new(
                        Point::new(x - 2.0, zone.rect.origin.y),
                        dock_core::Size::new(4.0, zone.rect.size.height),
                    );
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.10, false));
                    layer = layer.child(guide_rect(line, theme));
                }
                DropZone::ColumnEnd => {
                    let y = zone.rect.origin.y;
                    let line = Rect::new(
                        Point::new(zone.rect.origin.x, y - 2.0),
                        dock_core::Size::new(zone.rect.size.width, 4.0),
                    );
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.10, false));
                    layer = layer.child(guide_rect(line, theme));
                }
                DropZone::RailInsertion => {
                    if let Some(line) = rail_insertion_rect(zone, tab_headers) {
                        layer = layer.child(guide_rect(line, theme));
                    }
                }
                DropZone::FloatingCenter => {
                    if let Some(floating) = floating_for_target(
                        target,
                        groups,
                        floating_groups,
                    ) {
                        layer = layer.child(highlight_rect(
                            floating.rect,
                            theme,
                            0.20,
                            true,
                        ));
                    }
                }
                DropZone::FloatingTop | DropZone::FloatingBottom => {
                    if let Some(floating) = floating_for_target(
                        target,
                        groups,
                        floating_groups,
                    ) {
                        layer = layer.child(highlight_rect(
                            floating.rect,
                            theme,
                            0.08,
                            true,
                        ));
                    }
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.36, false));
                }
                DropZone::FreeArea => {
                    if let Some(bounds) = free_area_bounds {
                        layer = layer.child(highlight_rect(bounds, theme, 0.10, true));
                    }
                }
            }
        }

        layer
    }
}

impl DragGhost {
    pub(crate) fn render(
        preview: Option<DragPreview>,
        presentations: &HashMap<dock_core::NodeId, GroupPresentation>,
        floating_titles: &HashMap<FloatingGroupId, String>,
        viewport: Option<Rect>,
        theme: Theme,
    ) -> impl IntoElement {
        let Some(preview) = preview else {
            return div();
        };

        let (title, icon, subtitle, is_group) =
            ghost_presentation(preview.payload, presentations, floating_titles);
        let width = if is_group { 244.0 } else { 220.0 };
        let height = if subtitle.is_some() { 56.0 } else { 46.0 };
        let mut left = preview.point.x + 18.0;
        let mut top = preview.point.y + 18.0;
        if let Some(viewport) = viewport {
            left = left.min(viewport.origin.x + viewport.size.width - width);
            top = top.min(viewport.origin.y + viewport.size.height - height);
            left = left.max(viewport.origin.x);
            top = top.max(viewport.origin.y);
        }

        let mut icon_slot = div()
            .w(px(28.0))
            .h(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .bg(rgb(theme.colors.group_grip_background));

        if let Some(icon) = icon {
            icon_slot = icon_slot.child(img(icon).size(px(18.0)));
        } else {
            let fallback = title.chars().next().unwrap_or('•').to_string();
            icon_slot = icon_slot
                .text_color(rgb(theme.colors.primary_text))
                .text_size(px(theme.typography.tab_size))
                .child(fallback);
        }

        let mut text_column = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .justify_center()
            .overflow_hidden()
            .text_color(rgb(theme.colors.primary_text))
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(theme.typography.tab_size))
                    .child(title),
            );
        if let Some(subtitle) = subtitle {
            text_column = text_column.child(
                div()
                    .w_full()
                    .text_size(px((theme.typography.tab_size - 2.0).max(9.0)))
                    .text_color(rgb(theme.colors.muted_text))
                    .child(subtitle),
            );
        }

        // This view has no pointer listeners; it is purely visual and leaves
        // pointer interaction to the dock surfaces underneath it.
        div()
            .id("dock-drag-ghost")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .flex()
            .items_center()
            .gap(px(9.0))
            .px(px(9.0))
            .bg(rgb(theme.colors.panel_surface_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .shadow_lg()
            .opacity(0.94)
            .child(icon_slot)
            .child(text_column)
    }
}

fn ghost_presentation(
    payload: DragPayload,
    presentations: &HashMap<dock_core::NodeId, GroupPresentation>,
    floating_titles: &HashMap<FloatingGroupId, String>,
) -> (SharedString, Option<SharedString>, Option<SharedString>, bool) {
    match payload {
        DragPayload::Panel { panel } => {
            let tab = presentations
                .values()
                .find_map(|group| group.tabs.iter().find(|tab| tab.panel == panel));
            tab.map(|tab| {
                (
                    tab.title.clone(),
                    tab.icon.clone(),
                    Some(SharedString::from("Panel")),
                    false,
                )
            })
            .unwrap_or_else(|| {
                (
                    SharedString::from("Panel"),
                    None,
                    None,
                    false,
                )
            })
        }
        DragPayload::Group { group } => {
            let presentation = presentations.get(&group);
            let active = presentation.and_then(|presentation| {
                presentation
                    .tabs
                    .iter()
                    .find(|tab| Some(tab.panel) == presentation.active_panel)
                    .or_else(|| presentation.tabs.first())
            });
            let title = active
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| SharedString::from("Group"));
            let icon = active.and_then(|tab| tab.icon.clone());
            let count = presentation.map_or(0, |value| value.tabs.len());
            (
                title,
                icon,
                Some(SharedString::from(format!("{count} tabs"))),
                true,
            )
        }
        DragPayload::FloatingGroup { group } => (
            floating_titles
                .get(&group)
                .cloned()
                .map(SharedString::from)
                .unwrap_or_else(|| SharedString::from("Floating window")),
            None,
            Some(SharedString::from("Floating window")),
            true,
        ),
    }
}

fn target_group(target: DropTarget) -> Option<dock_core::NodeId> {
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

fn floating_for_target(
    target: DropTarget,
    groups: &[GroupRect],
    floating_groups: &[FloatingGroupRect],
) -> Option<FloatingGroupRect> {
    let group_id = target_group(target)?;
    let group = groups.iter().find(|group| group.group == group_id)?;
    let center = Point::new(
        group.rect.origin.x + group.rect.size.width * 0.5,
        group.rect.origin.y + group.rect.size.height * 0.5,
    );
    floating_groups
        .iter()
        .filter(|floating| {
            center.x >= floating.rect.origin.x
                && center.x <= floating.rect.origin.x + floating.rect.size.width
                && center.y >= floating.rect.origin.y
                && center.y <= floating.rect.origin.y + floating.rect.size.height
        })
        .max_by_key(|floating| floating.z_order)
        .copied()
}

fn group_edge_half(rect: Rect, zone: DropZone) -> Rect {
    match zone {
        DropZone::Left => Rect::new(
            rect.origin,
            dock_core::Size::new(rect.size.width * 0.5, rect.size.height),
        ),
        DropZone::Right => Rect::new(
            Point::new(rect.origin.x + rect.size.width * 0.5, rect.origin.y),
            dock_core::Size::new(rect.size.width * 0.5, rect.size.height),
        ),
        DropZone::Top => Rect::new(
            rect.origin,
            dock_core::Size::new(rect.size.width, rect.size.height * 0.5),
        ),
        DropZone::Bottom => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height * 0.5),
            dock_core::Size::new(rect.size.width, rect.size.height * 0.5),
        ),
        _ => rect,
    }
}

fn tab_insertion_rect(
    target: DropTarget,
    zone_rect: Rect,
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
                zone_rect.origin.x + zone_rect.size.width
            }
        });
    Some(Rect::new(
        Point::new(x - 1.5, strip.rect.origin.y),
        dock_core::Size::new(3.0, strip.rect.size.height),
    ))
}

fn rail_insertion_rect(
    zone: &DropZoneRect,
    headers: &[TabHeaderRect],
) -> Option<Rect> {
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
    let y = matching.map(|header| {
        let header_middle = header.rect.origin.y + header.rect.size.height * 0.5;
        if zone.rect.origin.y < header_middle {
            header.rect.origin.y
        } else {
            header.rect.origin.y + header.rect.size.height
        }
    }).unwrap_or(zone.rect.origin.y + zone.rect.size.height * 0.5);
    Some(Rect::new(
        Point::new(zone.rect.origin.x, y - 1.5),
        dock_core::Size::new(zone.rect.size.width, 3.0),
    ))
}

fn highlight_rect(
    rect: Rect,
    theme: Theme,
    opacity: f32,
    border: bool,
) -> impl IntoElement {
    let mut element = div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(theme.colors.drop_zone))
        .opacity(opacity);
    if border {
        element = element
            .border_1()
            .border_color(rgb(theme.colors.drop_zone_border));
    }
    element
}

fn guide_rect(rect: Rect, theme: Theme) -> impl IntoElement {
    div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(theme.colors.panel_tab_active))
}

#[cfg(test)]
mod tests {
    use dock_core::{DropZone, Point, Rect, Size};

    use super::group_edge_half;

    #[test]
    fn group_edge_preview_covers_half_of_the_target_group() {
        let rect = Rect::new(Point::new(10.0, 20.0), Size::new(200.0, 100.0));

        assert_eq!(
            group_edge_half(rect, DropZone::Left),
            Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 100.0))
        );
        assert_eq!(
            group_edge_half(rect, DropZone::Bottom),
            Rect::new(Point::new(10.0, 70.0), Size::new(200.0, 50.0))
        );
    }
}
