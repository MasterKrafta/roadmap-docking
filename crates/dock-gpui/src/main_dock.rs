use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DockSide, DragPreview,
    DropZoneRect, FlyoutState, GroupRect, NodeId, PanelRect, Point, Rect, Size, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled, div, px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    components::{
        context_menu::GroupMenuToggle,
        panel_host::PanelHost,
        rail::RailView,
        splitter::SplitterView,
    },
    dock_components::{
        active_panels_for_groups, CompositionHeader, DockGroupView, GroupPresentation,
    },
};

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
        _drag_preview: Option<DragPreview>,
        _drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        _middle_click_closes_tabs: bool,
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
        let mut layer = div()
            .id("composition-root")
            .absolute()
            .top_0()
            .left_0()
            .size_full();

        // Main owns one native window. Each Core Column is placed directly into
        // that window, with its Header rendered as a separate component below.
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
            let overflow = vertical_overflow
                .iter()
                .find(|overflow| overflow.owner == column.column)
                .copied();

            layer = layer.child(render_column_content(
                *column,
                group_ids,
                groups,
                &panels,
                mode,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                rail_display_orders
                    .get(&column.column)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                active_flyout,
                metrics,
                menu_group,
                context_menu.clone(),
                splitters,
                active_splitter,
                overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                None,
                true,
                dragged_group,
            ));
        }

        // Axis::X represents vertical dividers between Main Columns.
        // They are painted after the column content so their handles stay visible.
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

        // Every Main Column paints its own Header inside the shared Main HWND.
        if render_area_headers {
            for column in columns {
                let Some(side) = main_column_sides.get(&column.column).copied() else {
                    continue;
                };
                if dragged_dock_area == Some(side) {
                    continue;
                }

                layer = layer.child(CompositionHeader::render(
                    format!("main-column-header-{:?}", column.column),
                    column.rect,
                    std::slice::from_ref(column),
                    metrics,
                    theme,
                    false,
                    None,
                ));
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
                layer = layer.child(
                    div()
                        .id(format!("main-dock-area-{side_name}-drag-placeholder"))
                        .absolute()
                        .left(px(area_rect.origin.x))
                        .top(px(area_rect.origin.y))
                        .w(px(area_rect.size.width))
                        .h(px(area_rect.size.height))
                        .overflow_hidden()
                        .bg(rgb(theme.colors.dock_background))
                        .border_1()
                        .border_color(rgb(theme.colors.group_frame_border)),
                );
            }
        }

        layer
    }
}

/// Renders one Core Column's Groups, panel surfaces, rail or scroll viewport.
/// Header rendering stays with the host so Main can paint it in the same HWND
/// while Floating can put the exact same Header component in its own HWND.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_column_content(
    column: ColumnRect,
    column_group_ids: &[NodeId],
    groups: &[GroupRect],
    panels: &[(PanelRect, Option<AnyView>)],
    mode: ColumnMode,
    tab_strips: &[TabStripRect],
    tab_headers: &[TabHeaderRect],
    tab_overflows: &[TabOverflowRect],
    group_presentations: &HashMap<NodeId, GroupPresentation>,
    rail_display_order: &[ColumnDisplayItem],
    active_flyout: Option<FlyoutState>,
    metrics: DockMetrics,
    menu_group: Option<NodeId>,
    toggle_group_menu: GroupMenuToggle,
    splitters: &[SplitterRect],
    active_splitter: Option<(NodeId, usize, Axis)>,
    overflow: Option<VerticalOverflow>,
    scroll_handle: Option<ScrollHandle>,
    controller: Entity<DockController>,
    theme: Theme,
    raise_floating: Option<dock_core::FloatingGroupId>,
    show_header_space: bool,
    dragged_group: Option<NodeId>,
) -> impl IntoElement {
    let content_rect = Rect::new(
        Point::new(
            column.rect.origin.x,
            column.rect.origin.y + metrics.column_header_height,
        ),
        dock_core::Size::new(
            column.rect.size.width,
            (column.rect.size.height - metrics.column_header_height).max(0.0),
        ),
    );
    let header_offset = if show_header_space {
        0.0
    } else {
        metrics.column_header_height
    };
    let viewport = overflow.map_or(content_rect, |value| value.viewport);
    let content_extent = overflow.map_or(content_rect, |value| value.content_extent);
    let content_origin = content_extent.origin;

    let mut content = div()
        .relative()
        .w(px(content_extent.size.width))
        .h(px(content_extent.size.height));

    if mode == ColumnMode::Expanded {
        let mut ordered_groups = groups
            .iter()
            .filter(|group| column_group_ids.contains(&group.group))
            .copied()
            .collect::<Vec<_>>();
        ordered_groups.sort_by_key(|group| menu_group == Some(group.group));

        for group in &ordered_groups {
            if dragged_group == Some(group.group) {
                content = content.child(DockGroupView::render_placeholder(
                    *group,
                    content_origin,
                    metrics,
                    theme,
                ));
                continue;
            }

            let strip = tab_strips
                .iter()
                .find(|strip| strip.group == group.group)
                .copied();
            let presentation = group_presentations.get(&group.group);
            if let (Some(strip), Some(presentation)) = (strip, presentation) {
                let headers = tab_headers
                    .iter()
                    .filter(|header| header.group == group.group)
                    .copied()
                    .collect::<Vec<_>>();
                let tab_overflow = tab_overflows
                    .iter()
                    .find(|overflow| overflow.group == group.group)
                    .cloned();
                content = content.child(DockGroupView::render_core(
                    Some(column.column),
                    *group,
                    strip,
                    &headers,
                    tab_overflow,
                    presentation,
                    content_origin,
                    metrics,
                    0.0,
                    controller.clone(),
                    theme,
                    toggle_group_menu.clone(),
                ));
            } else {
                content = content.child(DockGroupView::render_placeholder(
                    *group,
                    content_origin,
                    metrics,
                    theme,
                ));
            }
        }

        // Axis::Y splitters belong to the Column's vertical Group stack.
        for splitter in splitters
            .iter()
            .filter(|splitter| splitter.owner == column.column && splitter.axis == Axis::Y)
        {
            content = content.child(SplitterView::render(
                *splitter,
                content_origin,
                controller.clone(),
                metrics,
                theme,
                raise_floating,
                active_splitter
                    == Some((splitter.owner, splitter.boundary_index, splitter.axis)),
            ));
        }

        let active_panels = active_panels_for_groups(
            column_group_ids.iter().copied(),
            group_presentations,
            dragged_group,
        );
        let column_panels = panels
            .iter()
            .filter(|(panel, _)| active_panels.contains(&panel.panel))
            .cloned()
            .collect::<Vec<_>>();
        content = content.child(PanelHost::render(column_panels, content_origin, theme));
    } else {
        let pointer_offset = Point::new(
            0.0,
            scroll_handle
                .as_ref()
                .map(|handle| handle.offset().y.as_f32().max(0.0))
                .unwrap_or(0.0),
        );
        content = content.child(RailView::render(
            column,
            mode,
            rail_display_order,
            tab_headers,
            group_presentations,
            content_origin,
            pointer_offset,
            active_flyout,
            controller.clone(),
            theme,
        ));
    }

    let viewport_top = viewport.origin.y - column.rect.origin.y - header_offset;
    let viewport_left = viewport.origin.x - column.rect.origin.x;
    let viewport_width = viewport.size.width;
    let viewport_height = viewport.size.height + header_offset;

    let mut scroll_viewport = div()
        .id(format!("dock-column-scroll-{:?}", column.column))
        .absolute()
        .top(px(viewport_top))
        .left(px(viewport_left))
        .w(px(viewport_width))
        .h(px(viewport_height))
        .bg(rgb(theme.colors.dock_background));

    if let Some(handle) = scroll_handle {
        scroll_viewport = scroll_viewport
            .overflow_y_scroll()
            .track_scroll(&handle)
            .child(content);
    } else {
        scroll_viewport = scroll_viewport.child(content);
    }

    div()
        .id(format!("dock-composition-column-{:?}", column.column))
        .absolute()
        .left(px(column.rect.origin.x))
        .top(px(column.rect.origin.y))
        .w(px(column.rect.size.width))
        .h(px(column.rect.size.height))
        .overflow_hidden()
        .child(
            div()
                .id(format!("dock-column-{:?}", column.column))
                .absolute()
                .left_0()
                .top_0()
                .w(px(column.rect.size.width))
                .h(px(column.rect.size.height))
                .overflow_hidden()
                .bg(rgb(theme.colors.dock_background))
                .child(scroll_viewport),
        )
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
