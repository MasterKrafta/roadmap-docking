use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DockSide, DragPreview,
    DropZoneRect, FlyoutState, GroupRect, NodeId, PanelRect, Point, Rect, Size, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{AnyView, Entity, IntoElement, ParentElement, ScrollHandle};
use ui::Theme;

use crate::{
    DockController,
    dock_components::{DockCompositionView, GroupPresentation},
};

use super::{context_menu::GroupMenuToggle, splitter::SplitterView};

/// Main-dock presentation layer. All geometry comes from a Core snapshot;
/// this component stores no second copy of the docking tree.
pub struct MainDockView;

impl MainDockView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: Vec<(PanelRect, Option<AnyView>)>,
        dragged_group: Option<NodeId>,
        dragged_dock_area: Option<DockSide>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        rail_display_orders: &HashMap<NodeId, Vec<ColumnDisplayItem>>,
        active_flyout: Option<FlyoutState>,
        drag_preview: Option<DragPreview>,
        drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        active_splitter: Option<(NodeId, usize, Axis)>,
        vertical_overflow: &[VerticalOverflow],
        main_node_ids: &HashSet<NodeId>,
        main_node_sides: &HashMap<NodeId, DockSide>,
        main_column_sides: &HashMap<NodeId, DockSide>,
        main_column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        scroll_handles: &HashMap<NodeId, ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        render_area_headers: bool,
    ) -> impl IntoElement {
        let mut layer = DockCompositionView::render_root();

        for column in columns {
            if dragged_dock_area == main_column_sides.get(&column.column).copied() {
                continue;
            }
            let group_ids = main_column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);

            layer = layer.child(crate::dock_components::DockCompositionView::render_column(
                *column,
                groups,
                &panels,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                group_ids,
                mode,
                rail_display_orders
                    .get(&column.column)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                active_flyout,
                drag_preview,
                drag_zones,
                metrics,
                menu_group,
                context_menu.clone(),
                middle_click_closes_tabs,
                splitters,
                active_splitter,
                vertical_overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                None,
                true,
                dragged_group,
            ));
        }

        // Axis::X represents vertical dividers between main-dock columns.
        // They are painted after columns so the handles remain visible.
        for splitter in splitters
            .iter()
            .filter(|splitter| {
                splitter.axis == Axis::X
                    && main_node_ids.contains(&splitter.owner)
                    && dragged_dock_area != main_node_sides.get(&splitter.owner).copied()
            })
        {
            layer = layer.child(SplitterView::render(
                *splitter,
                Point::ZERO,
                controller.clone(),
                metrics,
                theme,
                None,
                active_splitter
                    == Some((splitter.owner, splitter.boundary_index, splitter.axis)),
            ));
        }

        // Each Core DockArea (Left and Right) owns one shared header.
        // Column group geometry already starts below this reserved header band.
        if render_area_headers {
            for side in [DockSide::Left, DockSide::Right] {
                if dragged_dock_area == Some(side) {
                    continue;
                }
                let area_columns = columns
                    .iter()
                    .filter(|column| main_column_sides.get(&column.column) == Some(&side))
                    .copied()
                    .collect::<Vec<_>>();
                if let Some(area_rect) = dock_area_bounds(&area_columns) {
                    let side_name = match side {
                        DockSide::Left => "left",
                        DockSide::Right => "right",
                    };
                    layer = layer.child(DockCompositionView::render_header(
                        format!("main-dock-area-{side_name}"),
                        area_rect,
                        &area_columns,
                        metrics,
                        theme,
                    ));
                }
            }
        }

        if let Some(side) = dragged_dock_area {
            let area_columns = columns
                .iter()
                .filter(|column| main_column_sides.get(&column.column) == Some(&side))
                .copied()
                .collect::<Vec<_>>();
            if let Some(area_rect) = dock_area_bounds(&area_columns) {
                let side_name = match side {
                    DockSide::Left => "left",
                    DockSide::Right => "right",
                };
                layer = layer.child(DockCompositionView::render_drag_placeholder(
                    format!("main-dock-area-{side_name}-drag-placeholder"),
                    area_rect,
                    theme,
                ));
            }
        }

        layer
    }
}

pub(crate) fn dock_area_bounds(columns: &[ColumnRect]) -> Option<Rect> {
    let first = columns.first()?.rect;
    let mut left = first.origin.x;
    let mut top = first.origin.y;
    let mut right = first.origin.x + first.size.width;
    let mut bottom = first.origin.y + first.size.height;

    for column in columns.iter().skip(1) {
        left = left.min(column.rect.origin.x);
        top = top.min(column.rect.origin.y);
        right = right.max(column.rect.origin.x + column.rect.size.width);
        bottom = bottom.max(column.rect.origin.y + column.rect.size.height);
    }

    Some(Rect::new(
        Point::new(left, top),
        Size::new(right - left, bottom - top),
    ))
}
