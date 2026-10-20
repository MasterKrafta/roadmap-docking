use dock_core::{DockSide, DropTarget, DropZone, LayoutRects, Point, Rect, Size};
use gpui::{div, prelude::*, px, rgb};
use ui::Theme;

/// Paints the current valid drop target above the dock viewport.
///
/// The caller passes the result of resolve_drop, never the entire candidate-zone list.
/// Invalid targets and pointer locations outside a valid zone therefore produce no
/// indicator. All bounds use the logical-pixel coordinate space from LayoutRects.
/// This element is intended to be the final child of a relative dock viewport so the
/// dock panels themselves are not replaced by an overlay tree.
pub fn drop_overlay(
    theme: &Theme,
    rects: &LayoutRects,
    target: Option<&DropTarget>,
) -> impl IntoElement {
    let indicators = target
        .and_then(|target| {
            preview_indicators(
                rects,
                target,
                theme.metrics.drop_indicator_thickness.max(1.0),
                theme.metrics.drop_outline_thickness.max(1.0),
            )
        })
        .unwrap_or_default();
    let color = theme.colors.drop_target;

    if indicators.is_empty() {
        return div().w(px(0.0)).h(px(0.0));
    }

    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .w_full()
        .h_full()
        .children(indicators.into_iter().map(move |bounds| {
            div()
                .absolute()
                .left(px(bounds.origin.x))
                .top(px(bounds.origin.y))
                .w(px(bounds.size.width))
                .h(px(bounds.size.height))
                .bg(rgb(color))
        }))
}

fn preview_indicators(
    rects: &LayoutRects,
    target: &DropTarget,
    line_thickness: f32,
    outline_thickness: f32,
) -> Option<Vec<Rect>> {
    let indicators = match target {
        DropTarget::Group { group, zone } => {
            let group = rects.groups.iter().find(|item| item.group == *group)?;
            match zone {
                DropZone::Center => outline(group.bounds, outline_thickness),
                DropZone::Top => vec![horizontal_edge(group.bounds, true, line_thickness)],
                DropZone::Bottom => vec![horizontal_edge(group.bounds, false, line_thickness)],
                DropZone::Left => vec![vertical_edge(group.bounds, true, line_thickness)],
                DropZone::Right => vec![vertical_edge(group.bounds, false, line_thickness)],
            }
        }
        DropTarget::TabInsertion { group, index } => {
            let strip = rects.tab_strips.iter().find(|item| item.group == *group)?;
            let mut headers: Vec<_> = rects
                .tab_headers
                .iter()
                .filter(|item| item.group == *group)
                .collect();
            headers.sort_by_key(|item| item.index);
            let x = headers
                .iter()
                .find(|item| item.index == *index)
                .map(|item| item.bounds.origin.x)
                .or_else(|| {
                    headers
                        .last()
                        .map(|item| item.bounds.origin.x + item.bounds.size.width)
                })?;
            vec![Rect::new(
                Point::new(x - line_thickness * 0.5, strip.bounds.origin.y),
                Size::new(line_thickness, strip.bounds.size.height),
            )]
        }
        DropTarget::Column { column, zone } => {
            let column = rects
                .columns
                .iter()
                .find(|item| item.side == column.side && item.index == column.index)?;
            match zone {
                DropZone::Left => vec![vertical_edge(column.bounds, true, line_thickness)],
                DropZone::Right => vec![vertical_edge(column.bounds, false, line_thickness)],
                DropZone::Top => vec![horizontal_edge(column.bounds, true, line_thickness)],
                DropZone::Bottom => vec![horizontal_edge(column.bounds, false, line_thickness)],
                DropZone::Center => outline(column.bounds, outline_thickness),
            }
        }
        DropTarget::DockEdge { side } => {
            // An outer dock edge is also a valid destination when that side does not
            // have a DockArea yet, so derive the full viewport edge from current bounds.
            let viewport = viewport_bounds(rects)?;
            vec![vertical_edge(
                viewport,
                *side == DockSide::Left,
                line_thickness,
            )]
        }
        DropTarget::FloatingGroup { floating, .. } => {
            let floating = rects
                .floating_groups
                .iter()
                .find(|item| item.floating == *floating)?;
            outline(floating.bounds, outline_thickness)
        }
        DropTarget::FreeArea { bounds } => outline(*bounds, outline_thickness),
    };

    let indicators: Vec<_> = indicators
        .into_iter()
        .filter(|bounds| valid_rect(*bounds))
        .collect();
    (!indicators.is_empty()).then_some(indicators)
}

fn outline(bounds: Rect, thickness: f32) -> Vec<Rect> {
    if !valid_rect(bounds) {
        return Vec::new();
    }
    let thickness = thickness.min(bounds.size.width).min(bounds.size.height);
    vec![
        Rect::new(bounds.origin, Size::new(bounds.size.width, thickness)),
        Rect::new(
            Point::new(
                bounds.origin.x,
                bounds.origin.y + bounds.size.height - thickness,
            ),
            Size::new(bounds.size.width, thickness),
        ),
        Rect::new(bounds.origin, Size::new(thickness, bounds.size.height)),
        Rect::new(
            Point::new(
                bounds.origin.x + bounds.size.width - thickness,
                bounds.origin.y,
            ),
            Size::new(thickness, bounds.size.height),
        ),
    ]
}

fn horizontal_edge(bounds: Rect, top: bool, thickness: f32) -> Rect {
    let height = thickness.min(bounds.size.height.max(0.0));
    let y = if top {
        bounds.origin.y
    } else {
        bounds.origin.y + bounds.size.height - height
    };
    Rect::new(
        Point::new(bounds.origin.x, y),
        Size::new(bounds.size.width, height),
    )
}

fn vertical_edge(bounds: Rect, left: bool, thickness: f32) -> Rect {
    let width = thickness.min(bounds.size.width.max(0.0));
    let x = if left {
        bounds.origin.x
    } else {
        bounds.origin.x + bounds.size.width - width
    };
    Rect::new(
        Point::new(x, bounds.origin.y),
        Size::new(width, bounds.size.height),
    )
}

fn viewport_bounds(rects: &LayoutRects) -> Option<Rect> {
    let canvas = rects.canvas;
    if !canvas.origin.x.is_finite()
        || !canvas.origin.y.is_finite()
        || !canvas.size.width.is_finite()
        || !canvas.size.height.is_finite()
    {
        return None;
    }

    let mut left = canvas.origin.x;
    let mut top = canvas.origin.y;
    let mut right = canvas.origin.x + canvas.size.width.max(0.0);
    let mut bottom = canvas.origin.y + canvas.size.height.max(0.0);

    for area in &rects.dock_areas {
        if !valid_rect(area.bounds) {
            continue;
        }
        left = left.min(area.bounds.origin.x);
        top = top.min(area.bounds.origin.y);
        right = right.max(area.bounds.origin.x + area.bounds.size.width);
        bottom = bottom.max(area.bounds.origin.y + area.bounds.size.height);
    }

    let bounds = Rect::new(
        Point::new(left, top),
        Size::new((right - left).max(0.0), (bottom - top).max(0.0)),
    );
    valid_rect(bounds).then_some(bounds)
}

fn valid_rect(rect: Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
        && rect.size.width > 0.0
        && rect.size.height > 0.0
}

#[cfg(test)]
mod tests {
    use super::preview_indicators;
    use dock_core::{
        DockAreaRect, DockSide, DropTarget, DropZone, FloatingGroupId, GroupRect, LayoutRects,
        NodeId, Point, Rect, Size, TabHeaderRect, TabStripRect,
    };
    use slotmap::Key;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    #[test]
    fn group_center_draws_outline_and_edge_targets_draw_directional_lines() {
        let group_id = NodeId::null();
        let bounds = rect(100.0, 40.0, 280.0, 300.0);
        let rects = LayoutRects {
            groups: vec![GroupRect {
                group: group_id,
                bounds,
            }],
            ..LayoutRects::default()
        };

        let outline = preview_indicators(
            &rects,
            &DropTarget::Group {
                group: group_id,
                zone: DropZone::Center,
            },
            4.0,
            3.0,
        )
        .expect("known valid group should be previewable");
        assert_eq!(outline.len(), 4);
        assert_eq!(outline[0].size.height, 3.0);

        let split = preview_indicators(
            &rects,
            &DropTarget::Group {
                group: group_id,
                zone: DropZone::Bottom,
            },
            4.0,
            3.0,
        )
        .expect("group edge should have a line");
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].size.height, 4.0);
        assert_eq!(split[0].origin.y, 336.0);
    }

    #[test]
    fn tab_insertion_draws_a_vertical_line_between_headers() {
        let group = NodeId::null();
        let strip = rect(80.0, 32.0, 240.0, 28.0);
        let rects = LayoutRects {
            tab_strips: vec![TabStripRect {
                group,
                bounds: strip,
            }],
            tab_headers: vec![
                TabHeaderRect {
                    group,
                    index: 0,
                    bounds: rect(80.0, 32.0, 100.0, 28.0),
                },
                TabHeaderRect {
                    group,
                    index: 1,
                    bounds: rect(180.0, 32.0, 140.0, 28.0),
                },
            ],
            ..LayoutRects::default()
        };

        let line = preview_indicators(
            &rects,
            &DropTarget::TabInsertion { group, index: 1 },
            4.0,
            3.0,
        )
        .expect("valid tab insertion should be previewable");
        assert_eq!(line.len(), 1);
        assert_eq!(line[0].origin.x, 178.0);
        assert_eq!(line[0].size.width, 4.0);
        assert_eq!(line[0].size.height, 28.0);
    }

    #[test]
    fn missing_or_invalid_targets_do_not_create_an_overlay() {
        let rects = LayoutRects::default();
        assert!(
            preview_indicators(
                &rects,
                &DropTarget::Group {
                    group: NodeId::null(),
                    zone: DropZone::Center,
                },
                4.0,
                3.0,
            )
            .is_none()
        );

        assert!(
            preview_indicators(
                &rects,
                &DropTarget::FreeArea {
                    bounds: Rect::default(),
                },
                4.0,
                3.0,
            )
            .is_none()
        );
    }

    #[test]
    fn dock_edge_and_floating_targets_use_their_own_bounds() {
        let floating = FloatingGroupId::null();
        let dock_bounds = rect(0.0, 0.0, 260.0, 500.0);
        let floating_bounds = rect(120.0, 90.0, 360.0, 240.0);
        let rects = LayoutRects {
            dock_areas: vec![DockAreaRect {
                side: DockSide::Left,
                bounds: dock_bounds,
            }],
            floating_groups: vec![dock_core::FloatingGroupRect {
                floating,
                bounds: floating_bounds,
                z_order: 1,
            }],
            ..LayoutRects::default()
        };

        let dock_line = preview_indicators(
            &rects,
            &DropTarget::DockEdge {
                side: DockSide::Left,
            },
            4.0,
            3.0,
        )
        .expect("dock edge should be previewable");
        assert_eq!(dock_line[0].origin.x, 0.0);
        assert_eq!(dock_line[0].size.width, 4.0);

        let float_outline = preview_indicators(
            &rects,
            &DropTarget::FloatingGroup {
                floating,
                zone: DropZone::Center,
            },
            4.0,
            3.0,
        )
        .expect("floating group should be previewable");
        assert_eq!(float_outline.len(), 4);
        assert_eq!(float_outline[0].origin, floating_bounds.origin);
    }
}
