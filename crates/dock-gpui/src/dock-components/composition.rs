#[path = "composition_header.rs"]
mod header;
#[path = "group.rs"]
mod group;
#[path = "tab.rs"]
mod tab;

use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupId, FlyoutState, GroupRect, NodeId, PanelId, PanelRect, Point, Rect,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled, div, px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    components::{
        context_menu::GroupMenuToggle, panel_host::PanelHost, rail::RailView,
        splitter::SplitterView,
    },
    dock_components::GroupPresentation,
};

pub use group::{GroupDragArea, GroupGrip, GroupMenuButton, PanelContent};
pub use tab::DockTab;
pub use header::{
    CloseCompositionButton, CollapseColumnButton, CompositionDragArea, CompositionHeader,
};
pub(crate) use group::DockGroupView;

/// Renders one Column containing one or more vertically stacked Groups.
pub struct DockColumnView;

impl DockColumnView {
    /// Renders one Group occupying all available space below the Column header.
    pub fn one_group(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-one-group", 1, DockMetrics::default(), theme)
    }

    /// Renders two Groups stacked vertically with equal available height.
    pub fn two_groups(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-two-groups", 2, DockMetrics::default(), theme)
    }

    /// Renders a live Column with exactly one active tab and its panel view.
    ///
    /// Kept for small standalone previews. Runtime DockAreas use render_core so
    /// every Core Column shape follows the same layout and interaction path.
    pub fn render_active(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        Self::render_active_with_area_header(id, title, panel_view, metrics, theme, true)
    }

    /// Reserves the shared CompositionHeader space unless it is hidden during a DockTab-origin tear-off.
    ///
    /// While hidden, the Group occupies the full client area. On release the
    /// native window grows upward before the header is shown, keeping the tab
    /// row at the same screen position.
    pub(crate) fn render_active_with_area_header(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
        show_area_header: bool,
    ) -> impl IntoElement {
        let group_id = format!("{id}-group");
        let mut column = div()
            .id(id.clone())
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border));

        if show_area_header {
            column = column.child(
                div()
                    .id(format!("{id}-composition-header-space"))
                    .w_full()
                    .h(px(metrics.column_header_height))
                    .flex_none(),
            );
        }

        column.child(DockGroupView::render_active(
            group_id, title, panel_view, metrics, theme,
        ))
    }

    /// Renders a Core-owned Column using the same component path in Main and Floating windows.
    ///
    /// The caller places this view at the Column's Core rectangle. Group and
    /// panel geometry remains in the Core coordinate space; the local scroll
    /// viewport applies the corresponding origin/overflow translation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_core(
        column: ColumnRect,
        mode: ColumnMode,
        groups: &[GroupRect],
        panels: Vec<(PanelRect, Option<AnyView>)>,
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
        _middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        active_splitter: Option<(NodeId, usize, Axis)>,
        overflow: Option<VerticalOverflow>,
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        raise_floating: Option<FloatingGroupId>,
        show_area_header: bool,
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
                if dragged_group == Some(group.group) {
                    content = content.child(group_surface(*group, content_origin, metrics, theme));
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
                    active_splitter
                        == Some((splitter.owner, splitter.boundary_index, splitter.axis)),
                ));
            }

            // Active panel content is mounted at the exact rectangles returned by Core.
            content = content.child(PanelHost::render(panels, content_origin, theme));
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

        let viewport_top =
            viewport.origin.y - column.rect.origin.y - hidden_header_offset;
        let viewport_left = viewport.origin.x - column.rect.origin.x;
        let viewport_width = viewport.size.width;
        let viewport_height = viewport.size.height + hidden_header_offset;

        let mut scroll_viewport = div()
            .id(format!("dock-column-scroll-{:?}", column.column))
            .absolute()
            .top(px(viewport_top))
            .left(px(viewport_left))
            .w(px(viewport_width))
            .h(px(viewport_height))
            .bg(rgb(theme.colors.dock_background));

        if let Some(handle) = scroll_handle.as_ref() {
            scroll_viewport = scroll_viewport
                .overflow_y_scroll()
                .track_scroll(handle)
                .child(content);
        } else {
            scroll_viewport = scroll_viewport.child(content);
        }

        div()
            .id(format!("dock-column-{:?}", column.column))
            .absolute()
            .left_0()
            .top_0()
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .child(scroll_viewport)
    }

    fn render_groups(
        id: &'static str,
        group_count: usize,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let mut column = div()
            .id(id)
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(
                div()
                    .w_full()
                    .h(px(metrics.column_header_height))
                    .flex_none(),
            );

        for index in 0..group_count {
            let group_id = format!("{id}-group-{index}");
            column = column.child(DockGroupView::render_preview(group_id, metrics, theme));
        }

        column
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
    let content_height = (group.rect.size.height - header_height).max(0.0);

    div()
        .id(format!("dock-group-{:?}", group.group))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(group.rect.size.width))
        .h(px(group.rect.size.height))
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
        .child(
            div()
                .absolute()
                .top(px(header_height))
                .left_0()
                .w_full()
                .h(px(content_height))
                .bg(rgb(theme.colors.panel_surface_background)),
        )
}


/// Shared renderer for a Core composition.
///
/// MainDockView and FloatingWindowContent both delegate their Column surfaces
/// here. The host supplies the already host-local Core rectangles, floating
/// identity and CompositionHeader visibility; Column selection and component
/// routing stay identical in both window modes.
pub(crate) struct DockCompositionView;

impl DockCompositionView {
    /// Creates the root layer shared by Main and Floating compositions.
    pub(crate) fn render_root() -> impl IntoElement + InteractiveElement + ParentElement + Styled {
        div()
            .id("composition-root")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    /// Renders the visual placeholder left behind while a complete composition is transferred.
    pub(crate) fn render_drag_placeholder(
        id: String,
        area_rect: Rect,
        theme: Theme,
    ) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left(px(area_rect.origin.x))
            .top(px(area_rect.origin.y))
            .w(px(area_rect.size.width))
            .h(px(area_rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
    }

    /// Renders the one shared header above all Columns in this composition.
    pub(crate) fn render_header(
        id: String,
        area_rect: Rect,
        columns: &[ColumnRect],
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        CompositionHeader::render(id, area_rect, columns, metrics, theme)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_column(
        column: ColumnRect,
        groups: &[GroupRect],
        panels: &[(PanelRect, Option<AnyView>)],
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_group_ids: &[NodeId],
        column_mode: ColumnMode,
        rail_display_order: &[ColumnDisplayItem],
        active_flyout: Option<FlyoutState>,
        drag_preview: Option<DragPreview>,
        drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        menu_group: Option<NodeId>,
        context_menu: crate::components::context_menu::GroupMenuToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        active_splitter: Option<(NodeId, usize, Axis)>,
        vertical_overflow: &[VerticalOverflow],
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        raise_floating: Option<FloatingGroupId>,
        show_area_header: bool,
        dragged_group: Option<NodeId>,
    ) -> impl IntoElement {
        let group_rects = groups
            .iter()
            .filter(|group| column_group_ids.contains(&group.group))
            .copied()
            .collect::<Vec<_>>();
        let column_splitters = splitters
            .iter()
            .filter(|splitter| {
                splitter.owner == column.column && splitter.axis == dock_core::Axis::Y
            })
            .copied()
            .collect::<Vec<_>>();
        let overflow = vertical_overflow
            .iter()
            .find(|overflow| overflow.owner == column.column)
            .copied();
        let active_panels = column_group_ids
            .iter()
            .filter_map(|group| {
                group_presentations
                    .get(group)
                    .and_then(|presentation| presentation.active_panel)
            })
            .collect::<HashSet<PanelId>>();
        let column_panels = panels
            .iter()
            .filter(|(panel, _)| active_panels.contains(&panel.panel))
            .cloned()
            .collect::<Vec<_>>();

        // Every Core Column shape now enters DockColumnView. Its group/tab,
        // rail, splitter and panel-host children are shared by Main and Floating.
        let surface = div()
            .id(format!("dock-composition-column-{:?}", column.column))
            .absolute()
            .left(px(column.rect.origin.x))
            .top(px(column.rect.origin.y))
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            // Keep the surface absolute: this also establishes the containing
            // block for DockColumnView's local absolute coordinates.
            .overflow_hidden()
            .child(DockColumnView::render_core(
                column,
                column_mode,
                &group_rects,
                column_panels,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                rail_display_order,
                active_flyout,
                drag_preview,
                drag_zones,
                metrics,
                menu_group,
                context_menu,
                middle_click_closes_tabs,
                &column_splitters,
                active_splitter,
                overflow,
                scroll_handle,
                controller,
                theme,
                raise_floating,
                show_area_header,
                dragged_group,
            ));

        surface
    }
}
