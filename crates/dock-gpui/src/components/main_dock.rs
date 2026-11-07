use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DockSide, DragPreview, DropZoneRect,
    FlyoutState, GroupRect, NodeId, PanelRect, Point, Rect, Size, SplitterRect, TabHeaderRect, TabOverflowRect,
    TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, IntoElement, InteractiveElement, ParentElement, ScrollHandle, Styled, div, px,
};
use ui::Theme;

use crate::DockController;

use super::{
    column::ColumnView, context_menu::GroupMenuToggle, splitter::SplitterView,
    tab_strip::GroupPresentation,
};

/// Main-dock presentation layer. All geometry comes from a Core snapshot;
/// this component stores no second copy of the docking tree.
pub struct MainDockView;

impl MainDockView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: Vec<(PanelRect, AnyView)>,
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
        vertical_overflow: &[VerticalOverflow],
        main_node_ids: &HashSet<NodeId>,
        main_column_sides: &HashMap<NodeId, DockSide>,
        main_column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        scroll_handles: &HashMap<NodeId, ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut layer = crate::dock_components::DockAreaView::render_root();

        for column in columns {
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);
            let group_ids = main_column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let group_rects = groups
                .iter()
                .filter(|group| group_ids.contains(&group.group))
                .copied()
                .collect::<Vec<_>>();
            let group_splitters = splitters
                .iter()
                .filter(|splitter| splitter.owner == column.column && splitter.axis == Axis::Y)
                .copied()
                .collect::<Vec<_>>();
            let overflow = vertical_overflow
                .iter()
                .find(|overflow| overflow.owner == column.column)
                .copied();
            let scroll_handle = scroll_handles.get(&column.column).cloned();
            let active_panels = group_ids
                .iter()
                .filter_map(|group| {
                    group_presentations
                        .get(group)
                        .and_then(|presentation| presentation.active_panel)
                })
                .collect::<HashSet<_>>();
            let column_panels = panels
                .iter()
                .filter(|(panel, _)| active_panels.contains(&panel.panel))
                .cloned()
                .collect::<Vec<_>>();

            let single_active_panel = if group_rects.len() == 1 {
                group_rects.first().and_then(|group| {
                    let presentation = group_presentations.get(&group.group)?;
                    if presentation.tabs.len() != 1 {
                        return None;
                    }
                    let tab = presentation.tabs.first()?;
                    if presentation.active_panel != Some(tab.panel) {
                        return None;
                    }
                    let panel_view = column_panels
                        .iter()
                        .find(|(panel, _)| panel.panel == tab.panel)
                        .map(|(_, view)| view.clone())?;
                    Some((tab.title.to_string(), panel_view))
                })
            } else {
                None
            };

            if let Some((title, panel_view)) = single_active_panel {
                layer = layer.child(
                    div()
                        .id(format!("dock-components-column-{:?}", column.column))
                        .absolute()
                        .left(px(column.rect.origin.x))
                        .top(px(column.rect.origin.y))
                        .w(px(column.rect.size.width))
                        .h(px(column.rect.size.height))
                        .child(crate::dock_components::DockColumnView::render_active(
                            format!("dock-components-{:?}", column.column),
                            title,
                            panel_view,
                            metrics,
                            theme,
                        )),
                );
            } else {
                layer = layer.child(ColumnView::render(
                    *column,
                    mode,
                    &group_rects,
                    column_panels,
                    tab_strips,
                    tab_headers,
                    tab_overflows,
                    group_presentations,
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
                    &group_splitters,
                    overflow,
                    scroll_handle,
                    controller.clone(),
                    theme,
                    Point::ZERO,
                    None,
                    true,
                ));
            }
        }

        // Axis::X represents vertical dividers between main-dock columns.
        // They are painted after columns so the handles remain visible.
        for splitter in splitters
            .iter()
            .filter(|splitter| splitter.axis == Axis::X && main_node_ids.contains(&splitter.owner))
        {
            layer = layer.child(SplitterView::render(
                *splitter,
                Point::ZERO,
                Point::ZERO,
                controller.clone(),
                metrics,
                theme,
                None,
            ));
        }

        // Each Core DockArea (Left and Right) owns one shared header.
        // Column group geometry already starts below this reserved header band.
        for side in [DockSide::Left, DockSide::Right] {
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
                layer = layer.child(crate::dock_components::DockAreaView::render_header(
                    format!("main-dock-area-{side_name}"),
                    area_rect,
                    &area_columns,
                    metrics,
                    theme,
                ));
            }
        }

        layer
    }
}

fn dock_area_bounds(columns: &[ColumnRect]) -> Option<Rect> {
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
