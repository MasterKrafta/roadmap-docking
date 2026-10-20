use super::{HitResult, LayoutRects, Point, SplitterKind};

/// Resolve the interactive target under a point in logical layout coordinates.
///
/// Floating groups are tested from front to back, using the bottom-to-top order
/// provided by `compute_rects`. A floating group occludes every target below it,
/// including when the point falls into a non-interactive gap inside the float.
/// For the dock, splitters win over headers and panels; the Canvas is the final
/// fallback. All bounds use half-open edges: [x, x + width) × [y, y + height).
pub fn hit_test(point: Point, rects: &LayoutRects) -> HitResult {
    // LayoutRects stores floating groups bottom-to-top. A hit inside the first
    // matching group from the back must not fall through to groups below it.
    for floating in rects.floating_groups.iter().rev() {
        if !contains(floating.bounds, point) {
            continue;
        }

        if let Some(edge) = rects.floating_resize_edges.iter().find(|edge| {
            edge.floating == floating.floating && contains(edge.bounds, point)
        }) {
            return HitResult::FloatingResizeEdge {
                floating: floating.floating,
                edge: edge.edge,
            };
        }

        if rects.floating_titles.iter().any(|title| {
            title.floating == floating.floating && contains(title.bounds, point)
        }) {
            return HitResult::FloatingTitle(floating.floating);
        }

        if let Some(panel) = rects.panels.iter().find(|panel| {
            panel.floating == Some(floating.floating)
                && panel.active
                && contains(panel.bounds, point)
        }) {
            return HitResult::Panel(panel.panel);
        }

        // The topmost float owns this point even when its tab strip or another
        // non-interactive part has no dedicated HitResult.
        return HitResult::Empty;
    }

    // At splitter intersections, prefer the vertical column boundary over the
    // horizontal group boundary. Within one kind, compute_rects' stable index order
    // is retained.
    for kind in [SplitterKind::Column, SplitterKind::Group] {
        if let Some(splitter) = rects
            .splitters
            .iter()
            .find(|splitter| splitter.kind == kind && contains(splitter.bounds, point))
        {
            return HitResult::Splitter {
                kind: splitter.kind,
                index: splitter.index,
            };
        }
    }

    if let Some(header) = rects
        .column_headers
        .iter()
        .find(|header| contains(header.bounds, point))
    {
        return HitResult::ColumnHeader {
            side: header.side,
            index: header.index,
        };
    }

    if let Some(header) = rects
        .tab_headers
        .iter()
        .find(|header| contains(header.bounds, point))
    {
        return HitResult::TabHeader {
            group: header.group,
            index: header.index,
        };
    }

    if let Some(panel) = rects.panels.iter().find(|panel| {
        panel.floating.is_none() && panel.active && contains(panel.bounds, point)
    }) {
        return HitResult::Panel(panel.panel);
    }

    if contains(rects.canvas, point) {
        HitResult::Canvas
    } else {
        HitResult::Empty
    }
}

/// Test a non-empty rectangle with half-open right and bottom edges.
fn contains(bounds: super::Rect, point: Point) -> bool {
    let origin = bounds.origin;
    let size = bounds.size;
    if !point.x.is_finite()
        || !point.y.is_finite()
        || !origin.x.is_finite()
        || !origin.y.is_finite()
        || !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return false;
    }

    let x = f64::from(point.x);
    let y = f64::from(point.y);
    let left = f64::from(origin.x);
    let top = f64::from(origin.y);
    x >= left
        && x < left + f64::from(size.width)
        && y >= top
        && y < top + f64::from(size.height)
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::hit_test;
    use crate::{
        ColumnHeaderRect, DockSide, FloatingGroupId, FloatingGroupRect, FloatingResizeEdgeRect,
        FloatingTitleRect, HitResult, LayoutRects, PanelId, PanelRect, Point, Rect, ResizeEdge,
        Size, SplitterKind, SplitterRect, TabHeaderRect,
    };

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    fn panel(
        panel: PanelId,
        bounds: Rect,
        floating: Option<FloatingGroupId>,
        active: bool,
    ) -> PanelRect {
        PanelRect {
            panel,
            floating,
            active,
            bounds,
        }
    }

    fn floating_ids() -> (FloatingGroupId, FloatingGroupId, PanelId, PanelId) {
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let bottom = floating_ids.insert(());
        let top = floating_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let bottom_panel = panel_ids.insert(());
        let top_panel = panel_ids.insert(());
        (bottom, top, bottom_panel, top_panel)
    }

    #[test]
    fn rectangle_edges_are_half_open() {
        let panel_id = PanelId::null();
        let rects = LayoutRects {
            panels: vec![panel(panel_id, rect(10.0, 10.0, 10.0, 10.0), None, true)],
            ..LayoutRects::default()
        };

        assert_eq!(hit_test(Point::new(10.0, 10.0), &rects), HitResult::Panel(panel_id));
        assert_eq!(hit_test(Point::new(19.999, 19.999), &rects), HitResult::Panel(panel_id));
        assert_eq!(hit_test(Point::new(20.0, 15.0), &rects), HitResult::Empty);
        assert_eq!(hit_test(Point::new(15.0, 20.0), &rects), HitResult::Empty);
        assert_eq!(hit_test(Point::new(9.999, 15.0), &rects), HitResult::Empty);
    }

    #[test]
    fn floating_hits_precede_dock_and_canvas_and_use_frontmost_z_order() {
        let (bottom, top, bottom_panel, top_panel) = floating_ids();
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let rects = LayoutRects {
            canvas: bounds,
            floating_groups: vec![
                FloatingGroupRect {
                    floating: bottom,
                    bounds,
                    z_order: 1,
                },
                FloatingGroupRect {
                    floating: top,
                    bounds,
                    z_order: 2,
                },
            ],
            panels: vec![
                panel(bottom_panel, rect(0.0, 20.0, 100.0, 80.0), Some(bottom), true),
                panel(top_panel, rect(0.0, 20.0, 100.0, 80.0), Some(top), true),
                panel(PanelId::null(), bounds, None, true),
            ],
            floating_titles: vec![
                FloatingTitleRect {
                    floating: bottom,
                    bounds: rect(0.0, 0.0, 100.0, 20.0),
                },
                FloatingTitleRect {
                    floating: top,
                    bounds: rect(0.0, 0.0, 100.0, 20.0),
                },
            ],
            floating_resize_edges: vec![
                FloatingResizeEdgeRect {
                    floating: bottom,
                    edge: ResizeEdge::BottomRight,
                    bounds: rect(90.0, 90.0, 10.0, 10.0),
                },
                FloatingResizeEdgeRect {
                    floating: top,
                    edge: ResizeEdge::BottomRight,
                    bounds: rect(90.0, 90.0, 10.0, 10.0),
                },
            ],
            ..LayoutRects::default()
        };

        assert_eq!(
            hit_test(Point::new(50.0, 50.0), &rects),
            HitResult::Panel(top_panel)
        );
        assert_eq!(
            hit_test(Point::new(50.0, 10.0), &rects),
            HitResult::FloatingTitle(top)
        );
        assert_eq!(
            hit_test(Point::new(95.0, 95.0), &rects),
            HitResult::FloatingResizeEdge {
                floating: top,
                edge: ResizeEdge::BottomRight,
            }
        );
    }

    #[test]
    fn gap_inside_frontmost_float_blocks_click_through() {
        let (bottom, top, bottom_panel, top_panel) = floating_ids();
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let rects = LayoutRects {
            canvas: bounds,
            floating_groups: vec![
                FloatingGroupRect {
                    floating: bottom,
                    bounds,
                    z_order: 1,
                },
                FloatingGroupRect {
                    floating: top,
                    bounds,
                    z_order: 2,
                },
            ],
            panels: vec![
                panel(bottom_panel, rect(0.0, 20.0, 100.0, 80.0), Some(bottom), true),
                panel(top_panel, rect(0.0, 30.0, 100.0, 70.0), Some(top), true),
                panel(PanelId::null(), rect(0.0, 20.0, 100.0, 80.0), None, true),
            ],
            floating_titles: vec![
                FloatingTitleRect {
                    floating: bottom,
                    bounds: rect(0.0, 0.0, 100.0, 20.0),
                },
                FloatingTitleRect {
                    floating: top,
                    bounds: rect(0.0, 0.0, 100.0, 20.0),
                },
            ],
            ..LayoutRects::default()
        };

        assert_eq!(hit_test(Point::new(50.0, 25.0), &rects), HitResult::Empty);
    }

    #[test]
    fn dock_splitters_and_headers_precede_panels_and_canvas() {
        let panel_id = PanelId::null();
        let group = crate::NodeId::null();
        let overlap = rect(10.0, 10.0, 50.0, 50.0);
        let point = Point::new(20.0, 20.0);
        let panel_rect = panel(panel_id, overlap, None, true);

        let splitter_rects = LayoutRects {
            canvas: rect(0.0, 0.0, 100.0, 100.0),
            panels: vec![panel_rect],
            splitters: vec![SplitterRect {
                kind: SplitterKind::Group,
                index: 4,
                bounds: overlap,
            }],
            ..LayoutRects::default()
        };
        assert_eq!(
            hit_test(point, &splitter_rects),
            HitResult::Splitter {
                kind: SplitterKind::Group,
                index: 4
            }
        );

        let header_rects = LayoutRects {
            canvas: rect(0.0, 0.0, 100.0, 100.0),
            panels: vec![panel_rect],
            column_headers: vec![ColumnHeaderRect {
                side: DockSide::Right,
                index: 2,
                bounds: overlap,
            }],
            ..LayoutRects::default()
        };
        assert_eq!(
            hit_test(point, &header_rects),
            HitResult::ColumnHeader {
                side: DockSide::Right,
                index: 2
            }
        );

        let tab_rects = LayoutRects {
            canvas: rect(0.0, 0.0, 100.0, 100.0),
            panels: vec![panel_rect],
            tab_headers: vec![TabHeaderRect {
                group,
                index: 1,
                bounds: overlap,
            }],
            ..LayoutRects::default()
        };
        assert_eq!(
            hit_test(point, &tab_rects),
            HitResult::TabHeader { group, index: 1 }
        );
    }

    #[test]
    fn inactive_tab_content_is_not_a_hit_target() {
        let inactive = PanelId::null();
        let mut ids = SlotMap::<PanelId, ()>::with_key();
        let active = ids.insert(());
        let rects = LayoutRects {
            panels: vec![
                panel(inactive, rect(0.0, 0.0, 100.0, 100.0), None, false),
                panel(active, rect(0.0, 0.0, 100.0, 100.0), None, true),
            ],
            ..LayoutRects::default()
        };

        assert_eq!(hit_test(Point::new(50.0, 50.0), &rects), HitResult::Panel(active));
    }

    #[test]
    fn non_finite_points_never_hit_any_region() {
        let rects = LayoutRects {
            canvas: rect(0.0, 0.0, 100.0, 100.0),
            ..LayoutRects::default()
        };
        assert_eq!(hit_test(Point::new(f32::NAN, 10.0), &rects), HitResult::Empty);
        assert_eq!(hit_test(Point::new(10.0, f32::INFINITY), &rects), HitResult::Empty);
    }
}
