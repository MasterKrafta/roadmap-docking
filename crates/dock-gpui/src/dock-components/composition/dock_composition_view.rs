use std::collections::HashMap;

use dock_core::{
    Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupId, FlyoutState, GroupRect, NodeId, PanelRect, Rect, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, InteractiveElement, IntoElement, ParentElement, ScrollHandle, Styled, div,
    px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    dock_components::{active_panels_for_groups, GroupPresentation},
};

use super::{CloseCompositionAction, CompositionHeader, DockColumnView};

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

    /// Renders shared header chrome for the supplied Column(s).
    pub(crate) fn render_header(
        id: String,
        area_rect: Rect,
        columns: &[ColumnRect],
        metrics: DockMetrics,
        theme: Theme,
        show_close_button: bool,
        close_action: Option<CloseCompositionAction>,
    ) -> impl IntoElement {
        CompositionHeader::render(
            id,
            area_rect,
            columns,
            metrics,
            theme,
            show_close_button,
            close_action,
        )
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
