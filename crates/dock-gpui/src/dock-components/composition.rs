use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupId, FlyoutState, GroupRect, NodeId, PanelId, PanelRect,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Entity, IntoElement, InteractiveElement, ParentElement, ScrollHandle, Styled, div, px,
};
use ui::Theme;

use crate::{
    DockController,
    components::{column::ColumnView, context_menu::GroupMenuToggle, tab_strip::GroupPresentation},
};

use super::DockColumnView;

/// Shared renderer for a Core DockArea composition.
///
/// MainDockView and FloatingWindowContent both delegate their Column surfaces
/// here. The host supplies the already host-local Core rectangles, floating
/// identity and DockAreaHeader visibility; Column selection and component
/// routing stay identical in both window modes.
pub(crate) struct DockCompositionView;

impl DockCompositionView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_column(
        column: ColumnRect,
        groups: &[GroupRect],
        panels: &[(PanelRect, AnyView)],
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
        context_menu: GroupMenuToggle,
        middle_click_closes_tabs: bool,
        splitters: &[SplitterRect],
        vertical_overflow: &[VerticalOverflow],
        scroll_handle: Option<ScrollHandle>,
        controller: Entity<DockController>,
        theme: Theme,
        raise_floating: Option<FloatingGroupId>,
        show_area_header: bool,
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

        // Keep one visual path for both hosts. The compact DockColumn renderer
        // is used only for its supported single-group/single-tab shape; rail,
        // multi-group and multi-tab columns go through the full Core renderer.
        let single_active_panel = if column_mode == ColumnMode::Expanded
            && group_rects.len() == 1
        {
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

        // Give both branches the exact same Core-assigned Column rectangle.
        // Legacy ColumnView is translated into this local surface rather than
        // computing its position independently in Main and Floating hosts.
        let mut surface = div()
            .id(format!("dock-composition-column-{:?}", column.column))
            .absolute()
            .left(px(column.rect.origin.x))
            .top(px(column.rect.origin.y))
            .w(px(column.rect.size.width))
            .h(px(column.rect.size.height))
            .relative()
            .overflow_hidden();

        if let Some((title, panel_view)) = single_active_panel {
            surface = surface.child(DockColumnView::render_active_with_area_header(
                format!("dock-composition-{:?}", column.column),
                title,
                panel_view,
                metrics,
                theme,
                show_area_header,
            ));
        } else {
            surface = surface.child(ColumnView::render(
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
                overflow,
                scroll_handle,
                controller,
                theme,
                column.rect.origin,
                raise_floating,
                show_area_header,
            ));
        }

        surface
    }
}
