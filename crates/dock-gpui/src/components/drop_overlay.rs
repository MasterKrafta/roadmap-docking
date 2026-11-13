use std::collections::HashMap;

use dock_core::{
    ColumnRect, DragPreview, DropTarget, DropZone, DropZoneRect, DropZoneResolver, GroupRect,
    DockSide, NodeId, Point, Rect, TabHeaderRect, TabStripRect,
};
use gpui::{
    IntoElement, ParentElement, Styled, div,
    px, rgb,
};
use ui::Theme;

use super::drop_overlay_geometry::{
    centered_vertical_split_guide_rect, column_end_guide_rect, column_end_visual_rect,
    composition_edge_highlight_rect, floating_group_edge_guide_rect, floating_side_edge_guide_rect,
    group_edge_highlight_rect, group_edge_guide_rect, group_edge_visual_rect,
    main_dock_edge_guide_rect, main_group_edge_guide_rect, rail_insertion_rect,
    same_group_panel_tab_insertion_rect, tab_strip_group_highlight_rect, target_group,
};

/// Visual feedback for the current dock destination.
///
/// Main and Floating use the same visual treatment for equivalent resolved zones.
/// Floating same-group PanelTab reordering may use a geometry-only insertion marker.
pub struct DropOverlay;

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
        columns: &[ColumnRect],
        tab_headers: &[TabHeaderRect],
        tab_strips: &[TabStripRect],
        main_dock_bounds: Option<(&HashMap<DockSide, Rect>, &HashMap<NodeId, DockSide>)>,
        floating_bounds: Option<Rect>,
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
                    let guide = if let Some(bounds) = floating_bounds {
                        floating_side_edge_guide_rect(bounds, zone.rect, zone.zone)
                            .unwrap_or_else(|| group_edge_guide_rect(zone.rect, zone.zone))
                    } else {
                        main_group_edge_guide_rect(zone.rect, zone.zone, target_bounds)
                    };
                    layer = layer.child(guide_rect(guide, theme));
                }
                DropZone::Top | DropZone::Bottom => {
                    let target_bounds = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                        .map(|group| group.rect);
                    let visual_rect = group_edge_visual_rect(zone.rect, target_bounds);
                    layer = layer.child(highlight_rect(
                        group_edge_highlight_rect(visual_rect, zone.zone),
                        theme,
                        0.10,
                        false,
                    ));
                    layer = layer.child(guide_rect(
                        main_group_edge_guide_rect(visual_rect, zone.zone, target_bounds),
                        theme,
                    ));
                }
                DropZone::FloatingTop | DropZone::FloatingBottom => {
                    let target_bounds = target_group(target)
                        .and_then(|id| groups.iter().find(|group| group.group == id))
                        .map(|group| group.rect);
                    let visual_rect = group_edge_visual_rect(zone.rect, target_bounds);
                    layer = layer.child(highlight_rect(
                        group_edge_highlight_rect(visual_rect, zone.zone),
                        theme,
                        0.10,
                        false,
                    ));
                    layer = layer.child(guide_rect(
                        floating_group_edge_guide_rect(
                            visual_rect,
                            zone.zone,
                            floating_bounds,
                        ),
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
                    let visual_rect = match target {
                        DropTarget::ColumnSlot { column, .. } => columns
                            .iter()
                            .find(|rect| rect.column == column)
                            .map(|column| column_end_visual_rect(zone.rect, column.rect))
                            .unwrap_or(zone.rect),
                        _ => zone.rect,
                    };
                    let line = column_end_guide_rect(visual_rect);
                    layer = layer.child(highlight_rect(visual_rect, theme, 0.10, false));
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

fn zone_matches_preview(
    zone: &DropZoneRect,
    target: Option<DropTarget>,
    point: Point,
) -> bool {
    target == Some(zone.target) && zone.rect.contains(point).unwrap_or(false)
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

    use super::zone_matches_preview;
    use super::super::drop_overlay_geometry::{
        centered_vertical_split_guide_rect, column_end_guide_rect, column_end_visual_rect,
        composition_edge_highlight_rect, floating_group_edge_guide_rect,
        floating_side_edge_guide_rect, group_edge_guide_rect, group_edge_highlight_rect,
        group_edge_visual_rect, main_dock_edge_guide_rect, main_group_edge_guide_rect,
        rail_insertion_rect,
        same_group_panel_tab_insertion_rect, tab_strip_group_highlight_rect,
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
            group_edge_highlight_rect(top_zone, DropZone::Top),
            Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 10.0))
        );

        let bottom_zone = Rect::new(Point::new(10.0, 340.0), Size::new(200.0, 20.0));
        assert_eq!(
            group_edge_highlight_rect(bottom_zone, DropZone::Bottom),
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
            tab_strip_group_highlight_rect(target, &groups, &strips),
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
        let floating_bounds = Rect::new(Point::new(10.0, 40.0), Size::new(200.0, 340.0));

        assert_eq!(
            floating_group_edge_guide_rect(
                zone,
                DropZone::FloatingBottom,
                Some(floating_bounds),
            ),
            Rect::new(Point::new(10.0, 376.0), Size::new(200.0, 4.0))
        );
    }

    #[test]
    fn floating_bottom_guide_spans_the_full_target_group_width() {
        let target_group = Rect::new(Point::new(10.0, 60.0), Size::new(280.0, 270.0));
        let zone = Rect::new(Point::new(30.0, 336.0), Size::new(240.0, 24.0));
        let floating_bounds = Rect::new(Point::new(10.0, 40.0), Size::new(280.0, 340.0));
        let visual = group_edge_visual_rect(zone, Some(target_group));

        assert_eq!(visual.size.width, 280.0);
        assert_eq!(
            floating_group_edge_guide_rect(
                visual,
                DropZone::FloatingBottom,
                Some(floating_bounds),
            ),
            Rect::new(Point::new(10.0, 376.0), Size::new(280.0, 4.0))
        );
    }

    #[test]
    fn floating_top_highlight_keeps_the_shared_header_seam_treatment() {
        let top_zone = Rect::new(Point::new(10.0, 40.0), Size::new(200.0, 20.0));
        assert_eq!(
            group_edge_highlight_rect(top_zone, DropZone::FloatingTop),
            Rect::new(Point::new(10.0, 60.0), Size::new(200.0, 10.0))
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
                Point::new(48.0, 20.0),
                Size::new(4.0, 24.0),
            ))
        );
    }

    #[test]
    fn floating_left_and_right_guides_are_pinned_to_the_outer_window_edges() {
        let bounds = Rect::new(Point::new(100.0, 40.0), Size::new(300.0, 240.0));
        let left_zone = Rect::new(Point::new(100.5, 40.0), Size::new(16.0, 240.0));
        let right_zone = Rect::new(Point::new(384.5, 40.0), Size::new(16.0, 240.0));

        assert_eq!(
            floating_side_edge_guide_rect(bounds, left_zone, DropZone::Left),
            Some(Rect::new(Point::new(100.0, 40.0), Size::new(4.0, 240.0)))
        );
        assert_eq!(
            floating_side_edge_guide_rect(bounds, right_zone, DropZone::Right),
            Some(Rect::new(Point::new(396.0, 40.0), Size::new(4.0, 240.0)))
        );
    }

    #[test]
    fn floating_side_guides_do_not_override_group_local_generic_edges() {
        let bounds = Rect::new(Point::new(100.0, 40.0), Size::new(300.0, 240.0));
        let group_edge = Rect::new(Point::new(100.0, 90.0), Size::new(16.0, 80.0));
        assert_eq!(
            floating_side_edge_guide_rect(bounds, group_edge, DropZone::Left),
            None
        );
    }

    #[test]
    fn column_end_guide_spans_the_full_column_and_is_pinned_to_its_bottom() {
        let column = Rect::new(Point::new(20.0, 30.0), Size::new(260.0, 400.0));
        let zone = Rect::new(Point::new(32.0, 390.0), Size::new(236.0, 24.0));
        let visual = column_end_visual_rect(zone, column);
        assert_eq!(
            visual,
            Rect::new(Point::new(20.0, 390.0), Size::new(260.0, 24.0))
        );
        assert_eq!(
            column_end_guide_rect(visual),
            Rect::new(Point::new(20.0, 410.0), Size::new(260.0, 4.0))
        );
    }

    #[test]
    fn rail_insertion_guide_uses_the_shared_four_pixel_thickness() {
        let group = NodeId::null();
        let panel = PanelId::null();
        let header = TabHeaderRect::new(
            group,
            panel,
            0,
            Rect::new(Point::new(10.0, 20.0), Size::new(48.0, 24.0)),
        );
        let zone = DropZoneRect::new(
            DropTarget::RailSlot { column: NodeId::null(), index: 0 },
            DropZone::RailInsertion,
            Rect::new(Point::new(10.0, 20.0), Size::new(20.0, 12.0)),
        );
        assert_eq!(
            rail_insertion_rect(&zone, &[header]),
            Some(Rect::new(Point::new(10.0, 18.0), Size::new(20.0, 4.0)))
        );
    }

    #[test]
    fn column_splitter_guides_stay_centered_between_the_two_edges() {
        let splitter = Rect::new(Point::new(100.0, 20.0), Size::new(18.0, 300.0));
        assert_eq!(
            centered_vertical_split_guide_rect(splitter),
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
