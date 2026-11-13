use std::collections::HashMap;

use dock_core::{
    DragPayload, DragPreview, DropTarget, DropZone, DropZoneRect, DropZoneResolver, GroupRect,
    DockSide, NodeId, Point, Rect, TabHeaderRect, TabStripRect,
};
use gpui::{
    IntoElement, ParentElement, SharedString, Styled, div, img, prelude::InteractiveElement as _,
    px, rgb,
};
use ui::Theme;

use crate::dock_components::GroupPresentation;

/// Visual feedback for the current dock destination.
///
/// Main and Floating use the same visual treatment for equivalent resolved zones.
/// Floating same-group PanelTab reordering may use a geometry-only insertion marker.
pub struct DropOverlay;

/// A non-interactive visual card that follows the active drag pointer.
pub struct DragGhost;

impl DropOverlay {
    /// Resolves the preview zone from the resolver owned by the current host.
    ///
    /// Main and Floating hosts call the same path, but each supplies only its
    /// own window-local resolver snapshot.
    pub(crate) fn resolve_active_zone(
        resolver: Option<&DropZoneResolver>,
        preview: Option<DragPreview>,
    ) -> Option<DropZoneRect> {
        let preview = preview?;
        let zone = resolver?.resolve_zone(preview.point).ok()??;
        zone_matches_preview(&zone, preview.target, preview.point).then_some(zone)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        resolved_zone: Option<DropZoneRect>,
        preview: Option<DragPreview>,
        groups: &[GroupRect],
        tab_headers: &[TabHeaderRect],
        tab_strips: &[TabStripRect],
        main_dock_bounds: Option<(&HashMap<DockSide, Rect>, &HashMap<NodeId, DockSide>)>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut layer = div().absolute().top_0().left_0().size_full();
        let Some(preview) = preview else {
            return layer;
        };
        let Some(target) = preview.target else {
            return layer;
        };
        // Resolver-backed affordances are painted only when they still match
        // the preview. Same-group PanelTab reordering is handled by the fallback below.
        let Some(zone) = resolved_zone
            .filter(|zone| zone_matches_preview(zone, Some(target), preview.point))
        else {
            if main_dock_bounds.is_none()
                && let Some(line) =
                    same_group_panel_tab_insertion_rect(preview, target, tab_headers, tab_strips)
            {
                layer = layer.child(guide_rect(line, theme));
            }
            return layer;
        };

        {
            match zone.zone {
                DropZone::Center | DropZone::FloatingCenter => {
                    // The tab row is the visible affordance; don't shade the entire composition.
                }
                DropZone::TabInsertion => {
                    if let Some(strip_rect) =
                        tab_strip_group_highlight_rect(target, groups, tab_strips)
                    {
                        // Highlight the complete tab/control row. The Group
                        // rectangle is wider than TabStripRect when controls
                        // occupy the trailing part of the same header row.
                        layer = layer.child(highlight_rect(strip_rect, theme, 0.12, true));
                    }
                }
                DropZone::Left | DropZone::Right => {
                    // Floating outer sides already span the composition; Main
                    // generic group sides use the same local strip and guide.
                    let target_bounds = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                        .map(|group| group.rect);
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.10, false));
                    layer = layer.child(guide_rect(
                        main_group_edge_guide_rect(zone.rect, zone.zone, target_bounds),
                        theme,
                    ));
                }
                DropZone::Top
                | DropZone::Bottom
                | DropZone::FloatingTop
                | DropZone::FloatingBottom => {
                    // Use Main's header seam and outer-bottom treatment for both hosts.
                    let edge = canonical_group_edge_zone(zone.zone);
                    let target_bounds = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                        .map(|group| group.rect);
                    // Keep hit-testing geometry unchanged while extending
                    // every horizontal edge highlight to the full target Group.
                    let visual_rect = group_edge_visual_rect(zone.rect, target_bounds);
                    layer = layer.child(highlight_rect(
                        main_group_edge_highlight_rect(visual_rect, edge),
                        theme,
                        0.10,
                        false,
                    ));
                    layer = layer.child(guide_rect(
                        main_group_edge_guide_rect(visual_rect, zone.zone, target_bounds),
                        theme,
                    ));
                }
                DropZone::ColumnLeft | DropZone::ColumnRight => {
                    // Core resolves Photoshop side docking at Column scope; paint
                    // the guide across the corresponding Main DockArea's full height.
                    let visual_rect =
                        composition_edge_highlight_rect(zone, target, main_dock_bounds);
                    let line = centered_vertical_split_guide_rect(visual_rect);
                    layer = layer.child(highlight_rect(visual_rect, theme, 0.10, false));
                    layer = layer.child(guide_rect(line, theme));
                }
                DropZone::FloatingColumnInsertion => {
                    // The zone follows one internal Split(X) boundary, not the
                    // outer composition edge. Keep the marker inside that splitter.
                    let line = centered_vertical_split_guide_rect(zone.rect);
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.18, false));
                    layer = layer.child(guide_rect(line, theme));
                }
                DropZone::DockEdge => {
                    // Keep the full edge hit target, but place its visible guide
                    // close to the actual outer window border instead of centering
                    // the marker 12 px inside the main viewport.
                    if let DropTarget::MainColumn { side, .. } = target {
                        let line = main_dock_edge_guide_rect(zone.rect, side);
                        layer = layer.child(highlight_rect(zone.rect, theme, 0.10, false));
                        layer = layer.child(guide_rect(line, theme));
                    }
                }
                DropZone::ColumnEnd => {
                    let y = zone.rect.origin.y;
                    let line = Rect::new(
                        Point::new(
                            zone.rect.origin.x,
                            y - DROP_EDGE_GUIDE_THICKNESS * 0.5,
                        ),
                        dock_core::Size::new(
                            zone.rect.size.width,
                            DROP_EDGE_GUIDE_THICKNESS,
                        ),
                    );
                    layer = layer.child(highlight_rect(zone.rect, theme, 0.10, false));
                    layer = layer.child(guide_rect(line, theme));
                }
                DropZone::RailInsertion => {
                    if let Some(line) = rail_insertion_rect(&zone, tab_headers) {
                        layer = layer.child(guide_rect(line, theme));
                    }
                }
                // FreeArea moves/detaches a composition; CanvasSlot renders its bounds.
                DropZone::FreeArea => {}
            }
        }

        layer
    }
}

impl DragGhost {
    pub(crate) fn render(
        preview: Option<DragPreview>,
        presentations: &HashMap<dock_core::NodeId, GroupPresentation>,
        viewport: Option<Rect>,
        theme: Theme,
    ) -> impl IntoElement {
        let Some(preview) = preview else {
            return div().id("dock-drag-ghost");
        };

        let (title, icon, subtitle, is_group) =
            ghost_presentation(preview.payload, presentations);
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
) -> (
    SharedString,
    Option<SharedString>,
    Option<SharedString>,
    bool,
) {
    match payload {
        DragPayload::Panel { panel } => {
            let tab = presentations
                .values()
                .find_map(|group| group.tabs.iter().find(|tab| tab.panel == panel));
            tab.map(|tab| (tab.title.clone(), tab.icon.clone(), None, false))
                .unwrap_or_else(|| (SharedString::from("Panel"), None, None, false))
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
        DragPayload::DockArea { .. } => (
            SharedString::from("Dock area"),
            None,
            Some(SharedString::from("Dock composition")),
            true,
        ),
        DragPayload::FloatingGroup { .. } => (
            SharedString::from("Floating window"),
            None,
            Some(SharedString::from("Floating window")),
            true,
        ),
    }
}

fn zone_matches_preview(
    zone: &DropZoneRect,
    target: Option<DropTarget>,
    point: Point,
) -> bool {
    target == Some(zone.target) && zone.rect.contains(point).unwrap_or(false)
}

fn composition_edge_highlight_rect(
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

fn tab_strip_group_highlight_rect(
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

fn canonical_group_edge_zone(zone: DropZone) -> DropZone {
    match zone {
        DropZone::FloatingTop => DropZone::Top,
        DropZone::FloatingBottom => DropZone::Bottom,
        other => other,
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


const DROP_EDGE_GUIDE_THICKNESS: f32 = 4.0;

fn centered_vertical_split_guide_rect(rect: Rect) -> Rect {
    let center_x = rect.origin.x + rect.size.width * 0.5;
    Rect::new(
        Point::new(
            center_x - DROP_EDGE_GUIDE_THICKNESS * 0.5,
            rect.origin.y,
        ),
        dock_core::Size::new(DROP_EDGE_GUIDE_THICKNESS, rect.size.height),
    )
}

fn group_edge_guide_rect(rect: Rect, zone: DropZone) -> Rect {
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

fn group_edge_visual_rect(rect: Rect, target_group: Option<Rect>) -> Rect {
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

fn main_group_edge_highlight_rect(rect: Rect, zone: DropZone) -> Rect {
    match zone {
        // Keep the full width, but reduce the translucent highlight to half
        // the hit zone height below the composition header.
        DropZone::Top => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height),
            dock_core::Size::new(rect.size.width, rect.size.height * 0.5),
        ),
        _ => rect,
    }
}

fn main_group_edge_guide_rect(
    rect: Rect,
    zone: DropZone,
    target_group: Option<Rect>,
) -> Rect {
    let centered = || group_edge_guide_rect(rect, zone);

    match zone {
        DropZone::Top | DropZone::FloatingTop => {
            // The Top hit zone lives inside the bottom of the composition header.
            let y = rect.origin.y + rect.size.height - DROP_EDGE_GUIDE_THICKNESS * 0.5;
            Rect::new(
                Point::new(rect.origin.x, y),
                dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
            )
        }
        DropZone::Bottom | DropZone::FloatingBottom => {
            let zone_bottom = rect.origin.y + rect.size.height;
            let is_floating_bottom = zone == DropZone::FloatingBottom;
            let is_last_group = target_group.is_some_and(|group| {
                let group_bottom = group.origin.y + group.size.height;
                (zone_bottom - group_bottom).abs() <= 1.0
            });

            if is_floating_bottom || is_last_group {
                // The outer bottom stripe sits directly on the window/Group edge.
                let y = zone_bottom - DROP_EDGE_GUIDE_THICKNESS;
                Rect::new(
                    Point::new(rect.origin.x, y),
                    dock_core::Size::new(rect.size.width, DROP_EDGE_GUIDE_THICKNESS),
                )
            } else {
                // Internal separator: keep the stripe centered on the splitter.
                centered()
            }
        }
        _ => centered(),
    }
}

fn same_group_panel_tab_insertion_rect(
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

fn tab_insertion_rect(
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
        Point::new(x - 1.5, strip.rect.origin.y),
        dock_core::Size::new(3.0, strip.rect.size.height),
    ))
}

fn rail_insertion_rect(zone: &DropZoneRect, headers: &[TabHeaderRect]) -> Option<Rect> {
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
        Point::new(zone.rect.origin.x, y - 1.5),
        dock_core::Size::new(zone.rect.size.width, 3.0),
    ))
}

fn main_dock_edge_guide_rect(rect: Rect, side: DockSide) -> Rect {
    group_edge_guide_rect(
        rect,
        match side {
            DockSide::Left => DropZone::Left,
            DockSide::Right => DropZone::Right,
        },
    )
}

const DROP_ZONE_FILL: u32 = 0x168BFF;
const DROP_ZONE_BORDER: u32 = 0x65D5FF;
const DROP_ZONE_GUIDE: u32 = 0x35C8FF;

fn highlight_rect(rect: Rect, _theme: Theme, opacity: f32, border: bool) -> impl IntoElement {
    let mut element = div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(DROP_ZONE_FILL))
        .opacity(opacity.max(0.18));
    if border {
        element = element
            .border_1()
            .border_color(rgb(DROP_ZONE_BORDER));
    }
    element
}

fn guide_rect(rect: Rect, _theme: Theme) -> impl IntoElement {
    div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(DROP_ZONE_GUIDE))
}

#[cfg(test)]
mod tests {
    use dock_core::{
        DockSide, DragPayload, DragPreview, DropTarget, DropZone, DropZoneRect, NodeId,
        PanelId, Point, Rect, Size, TabHeaderRect, TabStripRect,
    };
    use slotmap::Key;

    use super::{
        composition_edge_highlight_rect, group_edge_guide_rect, main_dock_edge_guide_rect,
        main_group_edge_guide_rect, same_group_panel_tab_insertion_rect,
        group_edge_visual_rect, canonical_group_edge_zone, zone_matches_preview,
    };

    #[test]
    fn main_column_edge_highlight_spans_the_full_composition_for_group_and_panel_targets() {
        use std::collections::HashMap;

        let area = Rect::new(Point::new(20.0, 30.0), Size::new(420.0, 640.0));
        let area_bounds = HashMap::from([(DockSide::Left, area), (DockSide::Right, area)]);
        let group = NodeId::null();
        let node_sides = HashMap::from([(group, DockSide::Left)]);

        let group_zone = DropZoneRect::new(
            DropTarget::GroupEdge {
                group,
                zone: DropZone::Left,
            },
            DropZone::ColumnLeft,
            Rect::new(Point::new(80.0, 210.0), Size::new(12.0, 90.0)),
        );
        assert_eq!(
            composition_edge_highlight_rect(
                group_zone,
                group_zone.target,
                Some((&area_bounds, &node_sides)),
            ),
            Rect::new(Point::new(80.0, 30.0), Size::new(12.0, 640.0)),
        );

        let panel_zone = DropZoneRect::new(
            DropTarget::MainColumn {
                side: DockSide::Right,
                index: 1,
            },
            DropZone::ColumnRight,
            Rect::new(Point::new(400.0, 210.0), Size::new(12.0, 90.0)),
        );
        assert_eq!(
            composition_edge_highlight_rect(
                panel_zone,
                panel_zone.target,
                Some((&area_bounds, &node_sides)),
            ),
            Rect::new(Point::new(400.0, 30.0), Size::new(12.0, 640.0)),
        );
    }

    #[test]
    fn group_edge_guides_follow_the_center_of_the_resolved_drop_strip() {
        let rect = Rect::new(Point::new(10.0, 20.0), Size::new(200.0, 24.0));

        assert_eq!(
            group_edge_guide_rect(rect, DropZone::Top),
            Rect::new(Point::new(10.0, 30.0), Size::new(200.0, 4.0))
        );
        assert_eq!(
            group_edge_guide_rect(rect, DropZone::Bottom),
            Rect::new(Point::new(10.0, 30.0), Size::new(200.0, 4.0))
        );

        let side_rect = Rect::new(Point::new(10.0, 20.0), Size::new(24.0, 100.0));
        assert_eq!(
            group_edge_guide_rect(side_rect, DropZone::Left),
            Rect::new(Point::new(10.0, 20.0), Size::new(4.0, 100.0))
        );
        assert_eq!(
            group_edge_guide_rect(side_rect, DropZone::Right),
            Rect::new(Point::new(30.0, 20.0), Size::new(4.0, 100.0))
        );
    }

    #[test]
    fn main_top_edge_wide_highlight_extends_down_into_the_group() {
        let top_zone = Rect::new(Point::new(10.0, 40.0), Size::new(200.0, 20.0));
        assert_eq!(
            super::main_group_edge_highlight_rect(top_zone, DropZone::Top),
            Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 10.0))
        );

        let bottom_zone = Rect::new(Point::new(10.0, 340.0), Size::new(200.0, 20.0));
        assert_eq!(
            super::main_group_edge_highlight_rect(bottom_zone, DropZone::Bottom),
            bottom_zone
        );
    }

    #[test]
    fn main_group_edge_guides_align_with_the_header_seam_and_outer_bottom() {
        let top_zone = Rect::new(Point::new(10.0, 40.0), Size::new(200.0, 20.0));
        let target_group = Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 300.0));
        assert_eq!(
            main_group_edge_guide_rect(top_zone, DropZone::Top, Some(target_group)),
            Rect::new(Point::new(10.0, 58.0), Size::new(200.0, 4.0))
        );

        let bottom_zone = Rect::new(Point::new(10.0, 336.0), Size::new(200.0, 24.0));
        let target_group = Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 300.0));
        assert_eq!(
            main_group_edge_guide_rect(
                bottom_zone,
                DropZone::Bottom,
                Some(target_group),
            ),
            Rect::new(Point::new(10.0, 356.0), Size::new(200.0, 4.0))
        );

        let internal_zone = Rect::new(Point::new(10.0, 196.0), Size::new(200.0, 24.0));
        let upper_group = Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 140.0));
        assert_eq!(
            main_group_edge_guide_rect(
                internal_zone,
                DropZone::Bottom,
                Some(upper_group),
            ),
            Rect::new(Point::new(10.0, 206.0), Size::new(200.0, 4.0))
        );
    }

    #[test]
    fn floating_group_tab_insertion_highlights_the_full_tab_control_row() {
        let group = NodeId::null();
        let group_rect = Rect::new(Point::new(20.0, 40.0), Size::new(320.0, 280.0));
        // The visual strip ends before the two trailing Group controls.
        let strip_rect = Rect::new(Point::new(20.0, 68.0), Size::new(252.0, 24.0));
        let groups = [dock_core::GroupRect::new(group, group_rect)];
        let strips = [TabStripRect::new(group, strip_rect)];
        let target = DropTarget::GroupTabs { group, index: 4 };

        assert_eq!(
            super::tab_strip_group_highlight_rect(target, &groups, &strips),
            Some(Rect::new(Point::new(20.0, 68.0), Size::new(320.0, 24.0))),
            "the Floating append zone must highlight tabs and trailing controls"
        );
    }

    #[test]
    fn horizontal_edge_feedback_spans_the_target_group_without_expanding_hit_testing() {
        let target_group = Rect::new(Point::new(10.0, 52.0), Size::new(280.0, 260.0));
        for zone in [
            Rect::new(Point::new(30.0, 40.0), Size::new(240.0, 12.0)),
            Rect::new(Point::new(30.0, 180.0), Size::new(240.0, 12.0)),
            Rect::new(Point::new(30.0, 300.0), Size::new(240.0, 12.0)),
        ] {
            assert_eq!(
                group_edge_visual_rect(zone, Some(target_group)),
                Rect::new(
                    Point::new(10.0, zone.origin.y),
                    Size::new(280.0, zone.size.height),
                )
            );
            assert_eq!(
                zone.size.width,
                240.0,
                "visual stretching must not alter the Core hit area"
            );
        }
    }

    #[test]
    fn floating_bottom_guide_is_pinned_to_the_window_edge() {
        let zone = Rect::new(Point::new(10.0, 336.0), Size::new(200.0, 24.0));
        let target_group = Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 270.0));

        assert_eq!(
            super::main_group_edge_guide_rect(
                zone,
                DropZone::FloatingBottom,
                Some(target_group),
            ),
            Rect::new(Point::new(10.0, 356.0), Size::new(200.0, 4.0))
        );
    }

    #[test]
    fn floating_outer_edges_use_the_same_visual_semantics_as_main() {
        assert_eq!(
            canonical_group_edge_zone(DropZone::FloatingTop),
            DropZone::Top
        );
        assert_eq!(
            canonical_group_edge_zone(DropZone::FloatingBottom),
            DropZone::Bottom
        );
    }

    #[test]
    fn drop_feedback_requires_the_core_selected_target() {
        let zone = DropZoneRect::new(
            DropTarget::FreeArea,
            DropZone::FreeArea,
            Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0)),
        );

        assert!(zone_matches_preview(
            &zone,
            Some(DropTarget::FreeArea),
            Point::new(50.0, 60.0),
        ));
        assert!(!zone_matches_preview(
            &zone,
            Some(DropTarget::MainColumn {
                side: DockSide::Left,
                index: 0,
            }),
            Point::new(50.0, 60.0),
        ));
    }

    #[test]
    fn drop_feedback_disappears_when_the_pointer_leaves_the_resolved_zone() {
        let zone = DropZoneRect::new(
            DropTarget::FreeArea,
            DropZone::FreeArea,
            Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0)),
        );

        assert!(!zone_matches_preview(
            &zone,
            Some(DropTarget::FreeArea),
            Point::new(111.0, 60.0),
        ));
    }

    #[test]
    fn same_group_panel_tab_preview_shows_an_insertion_marker_without_a_drop_zone() {
        let group = NodeId::null();
        let panel = PanelId::null();
        let strip_rect = Rect::new(Point::new(10.0, 20.0), Size::new(180.0, 24.0));
        let headers = [TabHeaderRect::new(
            group,
            panel,
            0,
            Rect::new(Point::new(10.0, 20.0), Size::new(40.0, 24.0)),
        )];
        let strips = [TabStripRect::new(group, strip_rect)];
        let preview = DragPreview {
            payload: DragPayload::Panel { panel },
            point: Point::new(55.0, 30.0),
            target: Some(DropTarget::Tabs { group, index: 1 }),
        };

        assert_eq!(
            same_group_panel_tab_insertion_rect(
                preview,
                DropTarget::Tabs { group, index: 1 },
                &headers,
                &strips,
            ),
            Some(Rect::new(
                Point::new(48.5, 20.0),
                Size::new(3.0, 24.0),
            ))
        );
    }

    #[test]
    fn column_splitter_guides_stay_centered_between_the_two_edges() {
        let splitter = Rect::new(Point::new(100.0, 20.0), Size::new(18.0, 300.0));
        assert_eq!(
            super::centered_vertical_split_guide_rect(splitter),
            Rect::new(Point::new(107.0, 20.0), Size::new(4.0, 300.0))
        );
    }

    #[test]
    fn main_dock_edge_guides_stay_close_to_the_outer_window_border() {
        let left = Rect::new(Point::new(100.0, 20.0), Size::new(24.0, 300.0));
        let right = Rect::new(Point::new(976.0, 20.0), Size::new(24.0, 300.0));

        assert_eq!(
            main_dock_edge_guide_rect(left, DockSide::Left),
            Rect::new(Point::new(100.0, 20.0), Size::new(4.0, 300.0))
        );
        assert_eq!(
            main_dock_edge_guide_rect(right, DockSide::Right),
            Rect::new(Point::new(996.0, 20.0), Size::new(4.0, 300.0))
        );
    }
}
