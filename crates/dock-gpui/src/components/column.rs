use std::collections::HashMap;

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockCommand, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupId,
    FlyoutState, GroupRect, NodeId, PanelRect, Point, SplitterRect, TabHeaderRect, TabOverflowRect,
    TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, IntoElement, ParentElement, ScrollHandle, Styled, div, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::DockController;

use super::{
    context_menu::GroupMenuToggle,
    group::GroupView,
    panel_host::PanelHost,
    rail::RailView,
    splitter::SplitterView,
    tab_overflow::TabOverflowToggle,
    tab_strip::GroupPresentation,
};

/// One main-dock column. Core supplies the group rectangles and the scroll
/// extent; GPUI supplies only the clipping and ephemeral scroll position.
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
        drag_preview: Option<DragPreview>,
        drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
        overflow_group: Option<NodeId>,
        toggle_tab_overflow: TabOverflowToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        overflow: Option<VerticalOverflow>,
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        coordinate_origin: Point,
        raise_floating: Option<FloatingGroupId>,
    ) -> impl IntoElement {
        let viewport = overflow.map_or(column.rect, |value| value.viewport);
        let content_extent = overflow.map_or(column.rect, |value| value.content_extent);
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
                    let is_first_group = group.rect.origin.y == column.rect.origin.y;
                    let reserved_grip_left = if is_first_group {
                        metrics.group_control_button_width + 8.0
                    } else {
                        0.0
                    };
                    content = content.child(GroupView::render(
                        *group,
                        strip,
                        &headers,
                        overflow,
                        presentation,
                        content_origin,
                        metrics,
                        reserved_grip_left,
                        controller.clone(),
                        theme,
                        menu_group == Some(group.group),
                        toggle_group_menu.clone(),
                        overflow_group == Some(group.group),
                        toggle_tab_overflow.clone(),
                        middle_click_closes_tabs,
                    ));
                } else {
                    content = content.child(group_surface(*group, content_origin, theme));
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

            // Mount active panel content inside the scrolling content tree. This
            // keeps panel views aligned with Core geometry and clipped by the
            // owning column's scroll viewport.
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
                drag_preview,
                drag_zones,
                controller.clone(),
                theme,
            ));
        }

        let mut scroll_viewport = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(viewport.size.width))
            .h(px(viewport.size.height))
            .bg(rgb(theme.colors.dock_background));

        if let Some(handle) = scroll_handle.as_ref() {
            scroll_viewport = scroll_viewport
                .overflow_y_scroll()
                .track_scroll(handle)
                .child(content);
        } else {
            scroll_viewport = scroll_viewport.child(content);
        }

        let mut root = div()
            .id(format!("dock-column-{:?}", column.column))
            .absolute()
            .left(px(column.rect.origin.x - coordinate_origin.x))
            .top(px(column.rect.origin.y - coordinate_origin.y))
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .child(scroll_viewport);

        // The mode control sits in the column's top chrome band. It overlays
        // the first group grip rather than changing Core's authoritative rects.
        let next_mode = next_column_mode(mode);
        let mode_label = column_mode_label(mode);
        let controller_for_click = controller.clone();
        let column_id = column.column;
        let mode_button = div()
            .id(format!("column-mode-{:?}", column.column))
            .absolute()
            .top(px(3.0))
            .right(px(3.0))
            .w(px(18.0))
            .h(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.column_mode_button))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .child(mode_label)
            .on_click(move |_, _, app| {
                controller_for_click.update(app, |controller, controller_cx| {
                    if let Err(error) = controller.dispatch_in(
                        DockCommand::SetColumnMode {
                            column: column_id,
                            mode: next_mode,
                        },
                        controller_cx,
                    ) {
                        tracing::warn!(%error, "could not change dock column mode");
                    }
                });
            });

        root = root.child(mode_button);
        root
    }
}

fn next_column_mode(mode: ColumnMode) -> ColumnMode {
    match mode {
        ColumnMode::Expanded => ColumnMode::IconsAndLabels,
        ColumnMode::IconsAndLabels => ColumnMode::IconsOnly,
        ColumnMode::IconsOnly => ColumnMode::Expanded,
    }
}

fn column_mode_label(mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Expanded => "⟨⟨",
        ColumnMode::IconsAndLabels => "⟨",
        ColumnMode::IconsOnly => "⟩⟩",
    }
}

pub(crate) fn group_surface(group: GroupRect, content_origin: Point, theme: Theme) -> impl IntoElement {
    let left = group.rect.origin.x - content_origin.x;
    let top = group.rect.origin.y - content_origin.y;
    let header_height = theme
        .metrics
        .column_header_height
        .min(group.rect.size.height);

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

#[cfg(test)]
mod tests {
    use dock_core::ColumnMode;

    use super::next_column_mode;

    #[test]
    fn column_mode_button_cycles_through_all_three_modes() {
        assert_eq!(
            next_column_mode(ColumnMode::Expanded),
            ColumnMode::IconsAndLabels
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsAndLabels),
            ColumnMode::IconsOnly
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsOnly),
            ColumnMode::Expanded
        );
    }

}
