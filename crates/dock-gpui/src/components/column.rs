use std::collections::HashMap;

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupId, FlyoutState, GroupRect, NodeId, PanelRect, Point, Rect, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, IntoElement, ParentElement, ScrollHandle, StatefulInteractiveElement, Styled,
    div, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::DockController;

use super::{
    context_menu::GroupMenuToggle,
    group::GroupView,
    panel_host::PanelHost,
    rail::RailView,
    splitter::SplitterView,
    tab_strip::GroupPresentation,
};

/// One dock Column. Core supplies all Group rectangles, including adaptive
/// compact strips. GPUI keeps a scroll viewport only for flattened rail slots
/// or other explicit vertical overflow reported by Core.
pub struct ColumnView;

impl ColumnView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        column: ColumnRect,
        mode: ColumnMode,
        groups: &[GroupRect],
        panels: Vec<(PanelRect, AnyView)>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        rail_items: &[ColumnDisplayItem],
        active_flyout: Option<FlyoutState>,
        _drag_preview: Option<DragPreview>,
        _drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        overflow: Option<VerticalOverflow>,
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        coordinate_origin: Point,
        raise_floating: Option<FloatingGroupId>,
        show_area_header: bool,
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
        let hidden_header_offset = if show_area_header {
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
            let mut ordered_groups = groups.to_vec();
            ordered_groups.sort_by_key(|group| menu_group == Some(group.group));
            for group in &ordered_groups {
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
                    let overflow = tab_overflows
                        .iter()
                        .find(|overflow| overflow.group == group.group)
                        .cloned();
                    content = content.child(GroupView::render(
                        Some(column.column),
                        *group,
                        strip,
                        &headers,
                        overflow,
                        presentation,
                        content_origin,
                        metrics,
                        0.0,
                        controller.clone(),
                        theme,
                        raise_floating,
                        toggle_group_menu.clone(),
                        middle_click_closes_tabs,
                    ));
                } else {
                    content = content.child(group_surface(*group, content_origin, metrics, theme));
                }
            }
            let pointer_offset = Point::new(
                0.0,
                scroll_handle
                    .as_ref()
                    .map(|handle| handle.offset().y.as_f32().max(0.0))
                    .unwrap_or(0.0),
            );
            for splitter in splitters {
                content = content.child(SplitterView::render(
                    *splitter,
                    content_origin,
                    pointer_offset,
                    controller.clone(),
                    metrics,
                    theme,
                    raise_floating,
                ));
            }

            // Mount active panel content in the same coordinate space as Group
            // surfaces. Expanded Columns do not scroll; this viewport is only
            // scrollable when Core reports an explicit overflow (for example a rail).
            content = content.child(PanelHost::render(panels, content_origin));
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
                rail_items,
                tab_headers,
                group_presentations,
                content_origin,
                pointer_offset,
                active_flyout,
                controller.clone(),
                theme,
            ));
        }

        let mut scroll_viewport = div()
            .id(format!("dock-column-scroll-{:?}", column.column))
            .absolute()
            .top(px(
                viewport.origin.y - column.rect.origin.y - hidden_header_offset,
            ))
            .left(px(viewport.origin.x - column.rect.origin.x))
            .w(px(viewport.size.width))
            .h(px(viewport.size.height + hidden_header_offset))
            .bg(rgb(theme.colors.dock_background));

        if let Some(handle) = scroll_handle.as_ref() {
            scroll_viewport = scroll_viewport
                .overflow_y_scroll()
                .track_scroll(handle)
                .child(content);
        } else {
            scroll_viewport = scroll_viewport.child(content);
        }

        let root = div()
            .id(format!("dock-column-{:?}", column.column))
            .absolute()
            .left(px(column.rect.origin.x - coordinate_origin.x))
            .top(px(column.rect.origin.y - coordinate_origin.y))
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .child(scroll_viewport);

        root
    }
}

pub(crate) fn group_surface(
    group: GroupRect,
    content_origin: Point,
    metrics: DockMetrics,
    theme: Theme,
) -> impl IntoElement {
    let left = group.rect.origin.x - content_origin.x;
    let top = group.rect.origin.y - content_origin.y;
    let header_height = metrics.header_height.min(group.rect.size.height);

    div()
        .id(format!("dock-group-{:?}", group.group))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(group.rect.size.width))
        .h(px(group.rect.size.height))
        .bg(rgb(theme.colors.group_frame_background))
        .border_1()
        .border_color(rgb(theme.colors.group_frame_border))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(header_height))
                .bg(rgb(theme.colors.group_grip_background)),
        )
}
