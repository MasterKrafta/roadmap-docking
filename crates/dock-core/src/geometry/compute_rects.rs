use crate::geometry::{
    ColumnHeaderRect, ColumnRect, DockAreaRect, DockMetrics, FloatingGroupRect,
    FloatingResizeEdgeRect, FloatingTitleRect, GroupRect, LayoutRects, PanelRect, Point, Rect,
    Size, SplitterKind, SplitterRect, TabHeaderRect, TabStripRect, ResizeEdge,
};
use crate::model::{DockSide, Layout, Node};

/// Compute logical rectangles for the static dock, Canvas, and in-window floating groups.
///
/// Dock column widths are fixed by the model; the Canvas receives the remaining width.
 /// Group heights follow their ratios while respecting minimum heights. If minimums cannot
/// fit in the viewport, groups retain their minimum sizes and the column is marked as
/// vertically overflowing for a scrolling adapter. Floating groups are clamped to the
/// client bounds and returned separately in ascending z-order so they can be painted last.
///
/// This function does not depend on GPUI and does not round coordinates to physical pixels.
pub fn compute_rects(layout: &Layout, window: Size, metrics: &DockMetrics) -> LayoutRects {
    let window_width = nonnegative_finite(window.width);
    let window_height = nonnegative_finite(window.height);
    let header_height = nonnegative_finite(metrics.column_header_height).min(window_height);
    let tab_strip_height = nonnegative_finite(metrics.tab_strip_height);
    let splitter_thickness = nonnegative_finite(metrics.splitter_thickness);
    let splitter_hit_zone = nonnegative_finite(metrics.splitter_hit_zone);

    let left_width = side_width(layout, DockSide::Left);
    let right_width = side_width(layout, DockSide::Right);
    let canvas_x = left_width.min(window_width);
    let canvas_width = nonnegative_finite(window_width - left_width - right_width);

    let mut rects = LayoutRects {
        canvas: Rect::new(
            Point::new(canvas_x, 0.0),
            Size::new(canvas_width, window_height),
        ),
        ..LayoutRects::default()
    };

    let mut group_splitter_index = 0;
    let mut column_splitter_positions = Vec::new();

    if left_width > 0.0 {
        column_splitter_positions.push(left_width);
    }
    let canvas_right = nonnegative_finite(window_width - right_width);
    if right_width > 0.0 {
        column_splitter_positions.push(canvas_right);
    }

    for area in layout.dock_areas() {
        let area_width = area
            .columns
            .iter()
            .map(|column| nonnegative_finite(column.width))
            .fold(0.0, finite_saturating_add);
        let area_x = match area.side {
            DockSide::Left => 0.0,
            DockSide::Right => window_width - area_width,
        };
        rects.dock_areas.push(DockAreaRect {
            side: area.side,
            bounds: Rect::new(
                Point::new(area_x, 0.0),
                Size::new(area_width, window_height),
            ),
        });

        let mut column_x = area_x;
        for (column_index, column) in area.columns.iter().enumerate() {
            let column_width = nonnegative_finite(column.width);
            let column_bounds = Rect::new(
                Point::new(column_x, 0.0),
                Size::new(column_width, window_height),
            );

            let group_count = column.groups.len();
            let group_splitter_count = group_count.saturating_sub(1);
            let group_splitter_space = splitter_thickness * group_splitter_count as f32;
            let available_group_height =
                nonnegative_finite(window_height - header_height - group_splitter_space);
            let minimum_heights: Vec<f32> = column
                .groups
                .iter()
                .map(|group_id| minimum_group_height(layout, *group_id, metrics))
                .collect();
            let (group_heights, vertical_overflow) = allocate_group_heights(
                &column.group_ratios,
                &minimum_heights,
                available_group_height,
            );

            rects.columns.push(ColumnRect {
                side: area.side,
                index: column_index,
                bounds: column_bounds,
                vertical_overflow,
            });
            rects.column_headers.push(ColumnHeaderRect {
                side: area.side,
                index: column_index,
                bounds: Rect::new(
                    Point::new(column_x, 0.0),
                    Size::new(column_width, header_height),
                ),
            });

            if column_index + 1 < area.columns.len() {
                column_splitter_positions.push(column_x + column_width);
            }

            let mut y = header_height;
            for (group_index, group_id) in column.groups.iter().copied().enumerate() {
                let group_height = group_heights.get(group_index).copied().unwrap_or(0.0);
                let group_bounds = Rect::new(
                    Point::new(column_x, y),
                    Size::new(column_width, group_height),
                );
                rects.groups.push(GroupRect {
                    group: group_id,
                    bounds: group_bounds,
                });

                if let Some(Node::Tabs(tabs)) = layout.nodes().get(group_id) {
                    let strip_height = tab_strip_height.min(group_height);
                    let strip_bounds = Rect::new(
                        Point::new(column_x, y),
                        Size::new(column_width, strip_height),
                    );
                    rects.tab_strips.push(TabStripRect {
                        group: group_id,
                        bounds: strip_bounds,
                    });

                    let content_bounds = Rect::new(
                        Point::new(column_x, y + strip_height),
                        Size::new(column_width, nonnegative_finite(group_height - strip_height)),
                    );
                    for panel in &tabs.panels {
                        rects.panels.push(PanelRect {
                            panel: *panel,
                            bounds: content_bounds,
                        });
                    }

                    if !tabs.panels.is_empty() {
                        let tab_count = tabs.panels.len();
                        for (tab_index, _) in tabs.panels.iter().enumerate() {
                            let left = column_x
                                + column_width * tab_index as f32 / tab_count as f32;
                            let right = if tab_index + 1 == tab_count {
                                column_x + column_width
                            } else {
                                column_x
                                    + column_width * (tab_index + 1) as f32 / tab_count as f32
                            };
                            rects.tab_headers.push(TabHeaderRect {
                                group: group_id,
                                index: tab_index,
                                bounds: Rect::new(
                                    Point::new(left, y),
                                    Size::new(nonnegative_finite(right - left), strip_height),
                                ),
                            });
                        }
                    }
                }

                y += group_height;
                if group_index + 1 < group_count {
                    rects.splitters.push(SplitterRect {
                        kind: SplitterKind::Group,
                        index: group_splitter_index,
                        bounds: Rect::new(
                            Point::new(
                                column_x,
                                y + (splitter_thickness - splitter_hit_zone) * 0.5,
                            ),
                            Size::new(column_width, splitter_hit_zone),
                        ),
                    });
                    group_splitter_index += 1;
                    y += splitter_thickness;
                }
            }

            column_x += column_width;
        }
    }

    column_splitter_positions.sort_by(f32::total_cmp);
    for (index, x) in column_splitter_positions.into_iter().enumerate() {
        rects.splitters.push(SplitterRect {
            kind: SplitterKind::Column,
            index,
            bounds: Rect::new(
                Point::new(x - splitter_hit_zone * 0.5, 0.0),
                Size::new(splitter_hit_zone, window_height),
            ),
        });
    }

    // SlotMap iteration is not the paint order. Sort explicitly by z-order and append
    // all float-specific rectangles in that same order, leaving dock geometry underneath.
    let mut floating_groups: Vec<_> = layout.floating_groups().iter().collect();
    floating_groups.sort_by_key(|(_, group)| group.z_order);
    for (floating_id, floating) in floating_groups {
        let bounds = clamp_floating_bounds(
            layout,
            floating.bounds,
            &floating.panels,
            window_width,
            window_height,
            metrics,
        );
        rects.floating_groups.push(FloatingGroupRect {
            floating: floating_id,
            bounds,
            z_order: floating.z_order,
        });

        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            continue;
        }

        let title_height = nonnegative_finite(metrics.column_header_height).min(bounds.size.height);
        rects.floating_titles.push(FloatingTitleRect {
            floating: floating_id,
            bounds: Rect::new(bounds.origin, Size::new(bounds.size.width, title_height)),
        });

        let strip_height = nonnegative_finite(metrics.tab_strip_height)
            .min(nonnegative_finite(bounds.size.height - title_height));
        let panel_content_y = bounds.origin.y + title_height + strip_height;
        let panel_content_height =
            nonnegative_finite(bounds.size.height - title_height - strip_height);
        let panel_bounds = Rect::new(
            Point::new(bounds.origin.x, panel_content_y),
            Size::new(bounds.size.width, panel_content_height),
        );
        for panel in &floating.panels {
            rects.panels.push(PanelRect {
                panel: *panel,
                bounds: panel_bounds,
            });
        }

        append_floating_resize_edges(
            &mut rects,
            floating_id,
            bounds,
            splitter_hit_zone,
        );
    }

    rects
}

fn minimum_group_height(layout: &Layout, group_id: crate::NodeId, metrics: &DockMetrics) -> f32 {
    let mut minimum = nonnegative_finite(metrics.min_group_height);
    if let Some(Node::Tabs(tabs)) = layout.nodes().get(group_id) {
        for panel_id in &tabs.panels {
            if let Some(panel) = layout.panels().get(*panel_id) {
                minimum = minimum.max(nonnegative_finite(panel.constraints().min_size().height));
            }
        }
    }
    minimum
}

/// Allocate group heights by ratio, while enforcing per-group minima. When the sum of
/// minimums exceeds the viewport, minimum sizes win and the caller marks the column as
/// vertically overflowing instead of shrinking panels below their constraints.
fn allocate_group_heights(
    ratios: &[f32],
    minimums: &[f32],
    available: f32,
) -> (Vec<f32>, bool) {
    let count = minimums.len();
    if count == 0 {
        return (Vec::new(), false);
    }

    let available = nonnegative_finite(available);
    let minimum_total: f64 = minimums.iter().map(|height| f64::from(*height)).sum();
    if !minimum_total.is_finite() || minimum_total > f64::from(available) {
        return (minimums.to_vec(), true);
    }

    let mut weights: Vec<f64> = (0..count)
        .map(|index| {
            ratios
                .get(index)
                .copied()
                .filter(|ratio| ratio.is_finite() && *ratio >= 0.0)
                .map(f64::from)
                .unwrap_or(0.0)
        })
        .collect();
    let weight_sum: f64 = weights.iter().sum();
    if !weight_sum.is_finite() || weight_sum <= 0.0 {
        weights.fill(1.0);
    }

    let mut active = vec![true; count];
    let mut heights = vec![0.0_f64; count];
    let mut fixed_total = 0.0_f64;

    for _ in 0..count {
        let active_weight: f64 = weights
            .iter()
            .enumerate()
            .filter(|(index, _)| active[*index])
            .map(|(_, weight)| *weight)
            .sum();
        let active_count = active.iter().filter(|active| **active).count();
        if active_count == 0 {
            break;
        }

        let remaining = (f64::from(available) - fixed_total).max(0.0);
        let mut newly_fixed = Vec::new();
        for index in 0..count {
            if !active[index] {
                continue;
            }
            let share = if active_weight > 0.0 {
                remaining * weights[index] / active_weight
            } else {
                remaining / active_count as f64
            };
            if share < f64::from(minimums[index]) {
                newly_fixed.push(index);
            } else {
                heights[index] = share;
            }
        }

        if newly_fixed.is_empty() {
            break;
        }
        for index in newly_fixed {
            active[index] = false;
            heights[index] = f64::from(minimums[index]);
            fixed_total += f64::from(minimums[index]);
        }
    }

    heights
        .iter_mut()
        .enumerate()
        .for_each(|(index, height)| {
            if active[index] && *height == 0.0 {
                *height = f64::from(minimums[index]);
            }
        });

    let mut result: Vec<f32> = heights
        .into_iter()
        .map(|height| finite_f64_to_f32(height.max(0.0)))
        .collect();

    // Avoid tiny float-rounding deficits by assigning the remainder to the last
    // flexible group. This does not lower any group below its minimum.
    if let Some(last_active) = active.iter().rposition(|is_active| *is_active) {
        let other_total: f64 = result
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != last_active)
            .map(|(_, height)| f64::from(*height))
            .sum();
        let remainder = (f64::from(available) - other_total).max(f64::from(minimums[last_active]));
        result[last_active] = finite_f64_to_f32(remainder);
    }

    (result, false)
}

fn side_width(layout: &Layout, side: DockSide) -> f32 {
    layout
        .dock_areas()
        .iter()
        .filter(|area| area.side == side)
        .flat_map(|area| area.columns.iter())
        .map(|column| nonnegative_finite(column.width))
        .fold(0.0, finite_saturating_add)
}

fn clamp_floating_bounds(
    layout: &Layout,
    requested: Rect,
    panels: &[crate::PanelId],
    window_width: f32,
    window_height: f32,
    metrics: &DockMetrics,
) -> Rect {
    let minimum_width = minimum_floating_width(layout, panels, metrics);
    let minimum_height = minimum_floating_height(layout, panels, metrics);
    let width = clamped_dimension(requested.size.width, minimum_width, window_width);
    let height = clamped_dimension(requested.size.height, minimum_height, window_height);
    let max_x = nonnegative_finite(window_width - width);
    let max_y = nonnegative_finite(window_height - height);
    let x = finite_or_zero(requested.origin.x).clamp(0.0, max_x);
    let y = finite_or_zero(requested.origin.y).clamp(0.0, max_y);

    Rect::new(Point::new(x, y), Size::new(width, height))
}

fn minimum_floating_width(
    layout: &Layout,
    panels: &[crate::PanelId],
    metrics: &DockMetrics,
) -> f32 {
    let mut minimum = nonnegative_finite(metrics.default_column_width)
        .max(nonnegative_finite(metrics.min_column_width));
    for panel_id in panels {
        if let Some(panel) = layout.panels().get(*panel_id) {
            minimum = minimum.max(nonnegative_finite(panel.constraints().min_size().width));
        }
    }
    minimum
}

fn minimum_floating_height(
    layout: &Layout,
    panels: &[crate::PanelId],
    metrics: &DockMetrics,
) -> f32 {
    let mut minimum = nonnegative_finite(metrics.min_group_height);
    for panel_id in panels {
        if let Some(panel) = layout.panels().get(*panel_id) {
            minimum = minimum.max(nonnegative_finite(panel.constraints().min_size().height));
        }
    }
    minimum
}

fn clamped_dimension(requested: f32, minimum: f32, available: f32) -> f32 {
    if available <= 0.0 {
        return 0.0;
    }
    let desired = if requested.is_finite() && requested > 0.0 {
        requested.max(minimum)
    } else {
        minimum
    };
    nonnegative_finite(desired.min(available))
}

fn append_floating_resize_edges(
    rects: &mut LayoutRects,
    floating: crate::FloatingGroupId,
    bounds: Rect,
    requested_hit_zone: f32,
) {
    let width = nonnegative_finite(bounds.size.width);
    let height = nonnegative_finite(bounds.size.height);
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let hit = nonnegative_finite(requested_hit_zone).min(width).min(height);
    if hit <= 0.0 {
        return;
    }
    let corner_width = hit.min(width * 0.5);
    let corner_height = hit.min(height * 0.5);
    let inner_width = nonnegative_finite(width - corner_width * 2.0);
    let inner_height = nonnegative_finite(height - corner_height * 2.0);
    let right = bounds.origin.x + width;
    let bottom = bounds.origin.y + height;

    let corners = [
        (ResizeEdge::TopLeft, Rect::new(bounds.origin, Size::new(corner_width, corner_height))),
        (
            ResizeEdge::TopRight,
            Rect::new(
                Point::new(right - corner_width, bounds.origin.y),
                Size::new(corner_width, corner_height),
            ),
        ),
        (
            ResizeEdge::BottomLeft,
            Rect::new(
                Point::new(bounds.origin.x, bottom - corner_height),
                Size::new(corner_width, corner_height),
            ),
        ),
        (
            ResizeEdge::BottomRight,
            Rect::new(
                Point::new(right - corner_width, bottom - corner_height),
                Size::new(corner_width, corner_height),
            ),
        ),
    ];
    for (edge, edge_bounds) in corners {
        if edge_bounds.size.width > 0.0 && edge_bounds.size.height > 0.0 {
            rects.floating_resize_edges.push(FloatingResizeEdgeRect {
                floating,
                edge,
                bounds: edge_bounds,
            });
        }
    }

    let edges = [
        (
            ResizeEdge::Left,
            Rect::new(
                Point::new(bounds.origin.x, bounds.origin.y + corner_height),
                Size::new(hit, inner_height),
            ),
        ),
        (
            ResizeEdge::Top,
            Rect::new(
                Point::new(bounds.origin.x + corner_width, bounds.origin.y),
                Size::new(inner_width, hit),
            ),
        ),
        (
            ResizeEdge::Right,
            Rect::new(
                Point::new(right - hit, bounds.origin.y + corner_height),
                Size::new(hit, inner_height),
            ),
        ),
        (
            ResizeEdge::Bottom,
            Rect::new(
                Point::new(bounds.origin.x + corner_width, bottom - hit),
                Size::new(inner_width, hit),
            ),
        ),
    ];
    for (edge, edge_bounds) in edges {
        if edge_bounds.size.width > 0.0 && edge_bounds.size.height > 0.0 {
            rects.floating_resize_edges.push(FloatingResizeEdgeRect {
                floating,
                edge,
                bounds: edge_bounds,
            });
        }
    }
}

fn finite_saturating_add(left: f32, right: f32) -> f32 {
    finite_f64_to_f32((f64::from(left) + f64::from(right)).min(f64::from(f32::MAX)))
}

fn nonnegative_finite(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_f64_to_f32(value: f64) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        0.0
    } else {
        value.min(f64::from(f32::MAX)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::compute_rects;
    use crate::{
        DockCommand, DockMetrics, DockSide, FloatSource, FloatingGroupId, Layout, Node, PanelKey,
        Point, Rect, Size, SplitSide, SplitterKind,
    };

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    fn group_id(layout: &Layout, key: &str) -> crate::NodeId {
        layout
            .dock_areas()
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter().copied())
            .find(|group| {
                matches!(
                    layout.nodes().get(*group),
                    Some(Node::Tabs(tabs))
                        if tabs.panels.iter().any(|panel| layout.panels().get(*panel)
                            .is_some_and(|instance| instance.key().as_str() == key))
                )
            })
            .expect("panel group exists")
    }

    fn panel_id(layout: &Layout, key: &str) -> crate::PanelId {
        layout
            .panels()
            .iter()
            .find_map(|(id, panel)| (panel.key().as_str() == key).then_some(id))
            .expect("panel exists")
    }

    fn floating_id_for_panel(layout: &Layout, panel: crate::PanelId) -> FloatingGroupId {
        layout
            .floating_groups()
            .iter()
            .find_map(|(id, floating)| floating.panels.contains(&panel).then_some(id))
            .expect("floating group exists")
    }

    #[test]
    fn initial_photoshop_layout_matches_manual_1280_by_800_geometry() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let window = Size::new(1280.0, 800.0);
        let rects = compute_rects(&layout, window, &metrics);

        assert_eq!(rects.canvas, rect(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(rects.dock_areas.len(), 1);
        assert_eq!(rects.dock_areas[0].bounds, rect(1000.0, 0.0, 280.0, 800.0));
        assert_eq!(rects.dock_areas[0].side, DockSide::Right);
        assert_eq!(rects.columns.len(), 1);
        assert_eq!(rects.columns[0].bounds, rect(1000.0, 0.0, 280.0, 800.0));
        assert!(!rects.columns[0].vertical_overflow);
        assert_eq!(rects.column_headers[0].bounds, rect(1000.0, 0.0, 280.0, 32.0));

        let layers_group = group_id(&layout, "layers");
        let properties_group = group_id(&layout, "properties");
        assert_eq!(
            rects.groups,
            vec![
                super::super::GroupRect { group: layers_group, bounds: rect(1000.0, 32.0, 280.0, 382.0) },
                super::super::GroupRect { group: properties_group, bounds: rect(1000.0, 418.0, 280.0, 382.0) },
            ]
        );
        assert_eq!(
            rects.tab_strips,
            vec![
                super::super::TabStripRect { group: layers_group, bounds: rect(1000.0, 32.0, 280.0, 28.0) },
                super::super::TabStripRect { group: properties_group, bounds: rect(1000.0, 418.0, 280.0, 28.0) },
            ]
        );

        assert_eq!(rects.tab_headers.len(), 4);
        assert_eq!(
            rects.tab_headers.iter()
                .find(|header| header.group == layers_group && header.index == 0)
                .map(|header| header.bounds),
            Some(rect(1000.0, 32.0, 140.0, 28.0))
        );
        assert_eq!(
            rects.tab_headers.iter()
                .find(|header| header.group == layers_group && header.index == 1)
                .map(|header| header.bounds),
            Some(rect(1140.0, 32.0, 140.0, 28.0))
        );

        let panel_bounds = rect(1000.0, 60.0, 280.0, 354.0);
        for key in ["layers", "color"] {
            assert_eq!(
                rects.panels.iter().find(|panel| panel.panel == panel_id(&layout, key)).map(|panel| panel.bounds),
                Some(panel_bounds)
            );
        }
        for key in ["properties", "history"] {
            assert_eq!(
                rects.panels.iter().find(|panel| panel.panel == panel_id(&layout, key)).map(|panel| panel.bounds),
                Some(rect(1000.0, 446.0, 280.0, 354.0))
            );
        }

        let group_splitter = rects.splitters.iter().find(|splitter| splitter.kind == SplitterKind::Group).unwrap();
        assert_eq!(group_splitter.bounds, rect(1000.0, 412.0, 280.0, 8.0));
        let column_splitter = rects.splitters.iter().find(|splitter| splitter.kind == SplitterKind::Column).unwrap();
        assert_eq!(column_splitter.bounds, rect(996.0, 0.0, 8.0, 800.0));
        assert_eq!(compute_rects(&layout, window, &metrics), rects);
    }

    #[test]
    fn column_width_is_fixed_and_canvas_gets_remaining_space() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let rects = compute_rects(&layout, Size::new(1400.0, 800.0), &metrics);
        assert_eq!(rects.canvas.size.width, 1120.0);
        assert_eq!(rects.columns[0].bounds.size.width, 280.0);
    }

    #[test]
    fn groups_keep_their_minimum_height_when_the_window_is_too_small() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let rects = compute_rects(&layout, Size::new(1280.0, 100.0), &metrics);

        assert!(rects.columns[0].vertical_overflow);
        assert_eq!(rects.groups[0].bounds.size.height, 80.0);
        assert_eq!(rects.groups[1].bounds.size.height, 80.0);
        assert_eq!(rects.groups[0].bounds.size.height.min(rects.groups[1].bounds.size.height), 80.0);
        assert!(rects.splitters.iter().all(|splitter| splitter.bounds.size.width >= 0.0 && splitter.bounds.size.height >= 0.0));
    }

    #[test]
    fn floating_bounds_are_clamped_to_window_edges_and_painted_in_z_order() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let first_panel = panel_id(&layout, "layers");
        let second_panel = panel_id(&layout, "properties");
        let requested = rect(1200.0, 760.0, 320.0, 240.0);

        layout.apply(
            DockCommand::Float { source: FloatSource::Panel(first_panel), rect: requested },
            &metrics,
        ).expect("first panel should float");
        layout.apply(
            DockCommand::Float { source: FloatSource::Panel(second_panel), rect: requested },
            &metrics,
        ).expect("second panel should float");

        let first_id = floating_id_for_panel(&layout, first_panel);
        let second_id = floating_id_for_panel(&layout, second_panel);
        let rects = compute_rects(&layout, Size::new(1280.0, 800.0), &metrics);

        assert_eq!(rects.floating_groups.len(), 2);
        assert!(rects.floating_groups[0].z_order < rects.floating_groups[1].z_order);
        for floating in &rects.floating_groups {
            assert!(floating.bounds.origin.x >= 0.0);
            assert!(floating.bounds.origin.y >= 0.0);
            assert!(floating.bounds.size.width > 0.0);
            assert!(floating.bounds.size.height > 0.0);
            assert!(floating.bounds.origin.x + floating.bounds.size.width <= 1280.0);
            assert!(floating.bounds.origin.y + floating.bounds.size.height <= 800.0);
        }

        let first = rects.floating_groups.iter().find(|floating| floating.floating == first_id).unwrap();
        let second = rects.floating_groups.iter().find(|floating| floating.floating == second_id).unwrap();
        assert_eq!(first.bounds, rect(960.0, 560.0, 320.0, 240.0));
        assert_eq!(second.bounds, rect(960.0, 560.0, 320.0, 240.0));
        assert_eq!(rects.floating_titles.len(), 2);
        assert_eq!(rects.floating_resize_edges.len(), 16);
        assert!(rects.floating_titles.iter().all(|title| title.bounds.size.height == 32.0));
        assert!(rects.floating_resize_edges.iter().all(|edge| edge.bounds.size.width >= 0.0 && edge.bounds.size.height >= 0.0));
        assert_eq!(compute_rects(&layout, Size::new(1280.0, 800.0), &metrics), rects);
    }

    #[test]
    fn extreme_window_sizes_do_not_create_negative_rectangles() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let panel = panel_id(&layout, "layers");
        layout.apply(
            DockCommand::Float {
                source: FloatSource::Panel(panel),
                rect: rect(f32::INFINITY, f32::NAN, f32::MAX, f32::MAX),
            },
            &metrics,
        ).expect("normal floating command requires finite bounds, so this will be checked separately");
        let rects = compute_rects(&layout, Size::new(0.0, 0.0), &metrics);
        assert_eq!(rects.canvas.size, Size::new(0.0, 0.0));
        for bounds in std::iter::once(rects.canvas)
            .chain(rects.dock_areas.iter().map(|area| area.bounds))
            .chain(rects.columns.iter().map(|column| column.bounds))
            .chain(rects.groups.iter().map(|group| group.bounds))
            .chain(rects.floating_groups.iter().map(|group| group.bounds))
        {
            assert!(bounds.size.width.is_finite() && bounds.size.width >= 0.0);
            assert!(bounds.size.height.is_finite() && bounds.size.height >= 0.0);
        }
    }
}

#[cfg(test)]
mod allocation_tests {
    #[test]
    fn ratios_respect_minimums_when_only_one_group_would_underflow() {
        let (heights, overflow) = super::allocate_group_heights(&[0.01, 0.99], &[80.0, 80.0], 300.0);
        assert!(!overflow);
        assert_eq!(heights, vec![80.0, 220.0]);
    }

    #[test]
    fn shortage_uses_minimums_and_sets_vertical_overflow() {
        let (heights, overflow) = super::allocate_group_heights(&[0.5, 0.5], &[80.0, 80.0], 100.0);
        assert!(overflow);
        assert_eq!(heights, vec![80.0, 80.0]);
    }
}
