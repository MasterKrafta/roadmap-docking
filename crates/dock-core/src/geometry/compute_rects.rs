use crate::geometry::{
    ColumnHeaderRect, ColumnRect, DockAreaRect, DockMetrics, GroupRect, LayoutRects, PanelRect,
    Point, Rect, Size, SplitterKind, SplitterRect, TabHeaderRect, TabStripRect,
};
use crate::model::{DockSide, Layout, Node};

/// Compute the static dock and Canvas rectangles for a logical window size.
///
/// Column widths come from the model. The Canvas receives the remaining horizontal space.
/// Each dock column reserves its common header and its horizontal group splitters, then
/// distributes the remaining height using normalized group ratios. Floating groups are
/// intentionally omitted here and are handled by the later floating-geometry stage.
///
/// All coordinates remain logical pixels; this function does not round for device scale.
pub fn compute_rects(layout: &Layout, window: Size, metrics: &DockMetrics) -> LayoutRects {
    let left_width = side_width(layout, DockSide::Left);
    let right_width = side_width(layout, DockSide::Right);
    let canvas_width = window.width - left_width - right_width;

    let mut rects = LayoutRects {
        canvas: Rect::new(Point::new(left_width, 0.0), Size::new(canvas_width, window.height)),
        ..LayoutRects::default()
    };

    let mut group_splitter_index = 0;
    let mut column_splitter_positions = Vec::new();

    // Collect Canvas/dock borders and internal column borders first. Sorting gives
    // the column splitter index a deterministic left-to-right meaning.
    if left_width > 0.0 {
        column_splitter_positions.push(left_width);
    }
    let canvas_right = window.width - right_width;
    if right_width > 0.0 {
        column_splitter_positions.push(canvas_right);
    }

    for area in layout.dock_areas() {
        let area_width: f32 = area.columns.iter().map(|column| column.width).sum();
        let area_x = match area.side {
            DockSide::Left => 0.0,
            DockSide::Right => window.width - area_width,
        };
        rects.dock_areas.push(DockAreaRect {
            side: area.side,
            bounds: Rect::new(Point::new(area_x, 0.0), Size::new(area_width, window.height)),
        });

        let mut column_x = area_x;
        for (column_index, column) in area.columns.iter().enumerate() {
            let column_bounds = Rect::new(
                Point::new(column_x, 0.0),
                Size::new(column.width, window.height),
            );
            rects.columns.push(ColumnRect {
                side: area.side,
                index: column_index,
                bounds: column_bounds,
            });
            rects.column_headers.push(ColumnHeaderRect {
                side: area.side,
                index: column_index,
                bounds: Rect::new(
                    Point::new(column_x, 0.0),
                    Size::new(column.width, metrics.column_header_height),
                ),
            });

            if column_index + 1 < area.columns.len() {
                column_splitter_positions.push(column_x + column.width);
            }

            let group_count = column.groups.len();
            let group_splitter_count = group_count.saturating_sub(1);
            let group_splitter_space = metrics.splitter_thickness * group_splitter_count as f32;
            let group_space = window.height - metrics.column_header_height - group_splitter_space;
            let weights = normalized_weights(&column.group_ratios, group_count);
            let mut y = metrics.column_header_height;
            let mut allocated_group_height = 0.0;

            for (group_index, group_id) in column.groups.iter().copied().enumerate() {
                let is_last = group_index + 1 == group_count;
                // Assign the rounding remainder to the final group. This keeps the stack
                // bottom exactly aligned with the window bottom for ordinary finite input.
                let group_height = if is_last {
                    group_space - allocated_group_height
                } else {
                    let height = group_space * weights[group_index];
                    allocated_group_height += height;
                    height
                };
                let group_bounds = Rect::new(
                    Point::new(column_x, y),
                    Size::new(column.width, group_height),
                );
                rects.groups.push(GroupRect {
                    group: group_id,
                    bounds: group_bounds,
                });

                if let Some(Node::Tabs(tabs)) = layout.nodes().get(group_id) {
                    let strip_height = metrics.tab_strip_height;
                    let strip_bounds = Rect::new(
                        Point::new(column_x, y),
                        Size::new(column.width, strip_height),
                    );
                    rects.tab_strips.push(TabStripRect {
                        group: group_id,
                        bounds: strip_bounds,
                    });

                    let content_bounds = Rect::new(
                        Point::new(column_x, y + strip_height),
                        Size::new(column.width, group_height - strip_height),
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
                            let left = column_x + column.width * tab_index as f32 / tab_count as f32;
                            let right = if tab_index + 1 == tab_count {
                                column_x + column.width
                            } else {
                                column_x + column.width * (tab_index + 1) as f32 / tab_count as f32
                            };
                            rects.tab_headers.push(TabHeaderRect {
                                group: group_id,
                                index: tab_index,
                                bounds: Rect::new(
                                    Point::new(left, y),
                                    Size::new(right - left, strip_height),
                                ),
                            });
                        }
                    }
                }

                y += group_height;
                if !is_last {
                    rects.splitters.push(SplitterRect {
                        kind: SplitterKind::Group,
                        index: group_splitter_index,
                        bounds: Rect::new(
                            Point::new(
                                column_x,
                                y + (metrics.splitter_thickness - metrics.splitter_hit_zone) * 0.5,
                            ),
                            Size::new(column.width, metrics.splitter_hit_zone),
                        ),
                    });
                    group_splitter_index += 1;
                    y += metrics.splitter_thickness;
                }
            }
            column_x += column.width;
        }
    }

    column_splitter_positions.sort_by(f32::total_cmp);
    for (index, x) in column_splitter_positions.into_iter().enumerate() {
        rects.splitters.push(SplitterRect {
            kind: SplitterKind::Column,
            index,
            bounds: Rect::new(
                Point::new(x - metrics.splitter_hit_zone * 0.5, 0.0),
                Size::new(metrics.splitter_hit_zone, window.height),
            ),
        });
    }

    rects
}

fn side_width(layout: &Layout, side: DockSide) -> f32 {
    layout
        .dock_areas()
        .iter()
        .filter(|area| area.side == side)
        .flat_map(|area| area.columns.iter())
        .map(|column| column.width)
        .sum()
}

/// Convert possibly unnormalized model weights into proportions summing to one.
/// Validated layouts normally have positive finite ratios; equal shares are used as
/// a deterministic fallback for an empty, malformed, or all-zero ratio list.
fn normalized_weights(ratios: &[f32], count: usize) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }

    let mut weights: Vec<f32> = (0..count)
        .map(|index| {
            ratios
                .get(index)
                .copied()
                .filter(|ratio| ratio.is_finite() && *ratio >= 0.0)
                .unwrap_or(0.0)
        })
        .collect();
    let total: f32 = weights.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return vec![1.0 / count as f32; count];
    }

    for weight in &mut weights {
        *weight /= total;
    }
    weights
}

#[cfg(test)]
mod tests {
    use super::compute_rects;
    use crate::{
        DockMetrics, DockSide, Layout, Node, PanelKey, Point, Rect, Size, SplitterKind,
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
                            .is_some_and(|instance| instance.key() == &PanelKey::new(key).unwrap()))
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

    #[test]
    fn initial_photoshop_layout_matches_manual_1280_by_800_geometry() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let window = Size::new(1280.0, 800.0);
        let rects = compute_rects(&layout, window, &metrics);

        assert_eq!(rects.canvas, rect(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(rects.dock_areas.len(), 1);
        assert_eq!(
            rects.dock_areas[0].bounds,
            rect(1000.0, 0.0, 280.0, 800.0)
        );
        assert_eq!(rects.dock_areas[0].side, DockSide::Right);
        assert_eq!(rects.columns.len(), 1);
        assert_eq!(
            rects.columns[0].bounds,
            rect(1000.0, 0.0, 280.0, 800.0)
        );
        assert_eq!(rects.column_headers[0].bounds, rect(1000.0, 0.0, 280.0, 32.0));

        let layers_group = group_id(&layout, "layers");
        let properties_group = group_id(&layout, "properties");
        assert_eq!(
            rects.groups,
            vec![
                super::super::GroupRect {
                    group: layers_group,
                    bounds: rect(1000.0, 32.0, 280.0, 382.0),
                },
                super::super::GroupRect {
                    group: properties_group,
                    bounds: rect(1000.0, 418.0, 280.0, 382.0),
                },
            ]
        );

        assert_eq!(
            rects.tab_strips,
            vec![
                super::super::TabStripRect {
                    group: layers_group,
                    bounds: rect(1000.0, 32.0, 280.0, 28.0),
                },
                super::super::TabStripRect {
                    group: properties_group,
                    bounds: rect(1000.0, 418.0, 280.0, 28.0),
                },
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
        assert_eq!(
            rects.tab_headers.iter()
                .find(|header| header.group == properties_group && header.index == 0)
                .map(|header| header.bounds),
            Some(rect(1000.0, 418.0, 140.0, 28.0))
        );

        let panel_bounds = rect(1000.0, 60.0, 280.0, 354.0);
        assert_eq!(
            rects.panels
                .iter()
                .find(|panel| panel.panel == panel_id(&layout, "layers"))
                .map(|panel| panel.bounds),
            Some(panel_bounds)
        );
        assert_eq!(
            rects.panels
                .iter()
                .find(|panel| panel.panel == panel_id(&layout, "color"))
                .map(|panel| panel.bounds),
            Some(panel_bounds)
        );
        assert_eq!(
            rects.panels
                .iter()
                .find(|panel| panel.panel == panel_id(&layout, "properties"))
                .map(|panel| panel.bounds),
            Some(rect(1000.0, 446.0, 280.0, 354.0))
        );
        assert_eq!(
            rects.panels
                .iter()
                .find(|panel| panel.panel == panel_id(&layout, "history"))
                .map(|panel| panel.bounds),
            Some(rect(1000.0, 446.0, 280.0, 354.0))
        );

        let group_splitters: Vec<_> = rects
            .splitters
            .iter()
            .filter(|splitter| splitter.kind == SplitterKind::Group)
            .collect();
        assert_eq!(group_splitters.len(), 1);
        assert_eq!(group_splitters[0].index, 0);
        assert_eq!(group_splitters[0].bounds, rect(1000.0, 412.0, 280.0, 8.0));
        let column_splitters: Vec<_> = rects
            .splitters
            .iter()
            .filter(|splitter| splitter.kind == SplitterKind::Column)
            .collect();
        assert_eq!(column_splitters.len(), 1);
        assert_eq!(column_splitters[0].index, 0);
        assert_eq!(column_splitters[0].bounds, rect(996.0, 0.0, 8.0, 800.0));

        // Repeated calls are deterministic down to every logical coordinate.
        assert_eq!(compute_rects(&layout, window, &metrics), rects);
    }

    #[test]
    fn column_width_is_fixed_and_canvas_gets_remaining_space() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let first = layout.dock_areas()[0].columns[0].width;
        assert_eq!(first, 280.0);

        let rects = compute_rects(&layout, Size::new(1400.0, 800.0), &metrics);
        assert_eq!(rects.canvas.size.width, 1120.0);
        assert_eq!(rects.columns[0].bounds.size.width, first);
    }

    #[test]
    fn ratios_are_normalized_before_group_heights_are_distributed() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).expect("initial layout");
        let rects = compute_rects(&layout, Size::new(1280.0, 800.0), &metrics);
        assert_eq!(rects.groups[0].bounds.size.height, 382.0);
        assert_eq!(rects.groups[1].bounds.size.height, 382.0);
        assert_eq!(rects.groups[0].bounds.origin.y, 32.0);
        assert_eq!(rects.groups[1].bounds.origin.y, 418.0);
    }
}

#[cfg(test)]
mod ratio_tests {
    #[test]
    fn normalizes_arbitrary_group_weights_to_proportions() {
        assert_eq!(super::normalized_weights(&[1.0, 3.0], 2), vec![0.25, 0.75]);
    }
}
