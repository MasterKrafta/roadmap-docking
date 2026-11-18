use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupRect, FlyoutState, GroupRect, NodeId, PanelRect, Point, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, App, Entity, IntoElement, ParentElement, Styled, WeakEntity, div,
    prelude::InteractiveElement as _, rgb,
};
use ui::Theme;

use crate::{
    components::main_dock::render_column_content,
    dock_components::{
        active_panels_for_groups, CompositionHeader, DockGroupView, GroupPresentation,
    },
    DockController, FloatingWindowManager,
};
use super::super::{
    context_menu::GroupMenuToggle,
    panel_host::PanelHost,
    splitter::SplitterView,
};

/// Content view for one legacy whole-composition Floating window.
///
/// Permanent Floating Headers and Groups use FloatingPartHost. This renderer
/// remains for the transitional whole-window host but shares the same
/// independent Header and Group components as Main.
pub(super) struct FloatingWindowContent;

impl FloatingWindowContent {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        floating: FloatingGroupRect,
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: Vec<(PanelRect, Option<AnyView>)>,
        dragged_group: Option<NodeId>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        rail_display_orders: &HashMap<NodeId, Vec<ColumnDisplayItem>>,
        splitters: &[SplitterRect],
        active_splitter: Option<(NodeId, usize, dock_core::Axis)>,
        vertical_overflow: &[VerticalOverflow],
        scroll_handles: &HashMap<NodeId, gpui::ScrollHandle>,
        active_flyout: Option<FlyoutState>,
        _drag_preview: Option<DragPreview>,
        _drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        hide_header: bool,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        _middle_click_closes_tabs: bool,
        controller: Entity<DockController>,
        manager: WeakEntity<FloatingWindowManager>,
        theme: Theme,
    ) -> impl IntoElement {
        let origin = Point::ZERO;
        let mut column_group_ids = HashSet::new();
        for ids in column_groups.values() {
            column_group_ids.extend(ids.iter().copied());
        }

        let column_active_panels = active_panels_for_groups(
            column_group_ids.iter().copied(),
            group_presentations,
            dragged_group,
        );
        let dragged_group_panels = dragged_group
            .and_then(|group| group_presentations.get(&group))
            .map(|presentation| {
                presentation
                    .tabs
                    .iter()
                    .map(|tab| tab.panel)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        let mut root = div()
            .id(format!("floating-window-{:?}", floating.group))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background));

        for column in columns {
            let group_ids = column_groups
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

            root = root.child(render_column_content(
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
                Some(floating.group),
                !hide_header,
                dragged_group,
            ));
        }

        // Standalone Tabs nodes outside Columns still use the same Group component.
        for group in groups
            .iter()
            .filter(|group| !column_group_ids.contains(&group.group))
        {
            let Some(strip) = tab_strips
                .iter()
                .find(|strip| strip.group == group.group)
                .copied()
            else {
                continue;
            };
            let Some(presentation) = group_presentations.get(&group.group) else {
                continue;
            };
            let headers = tab_headers
                .iter()
                .filter(|header| header.group == group.group)
                .copied()
                .collect::<Vec<_>>();
            let overflow = tab_overflows
                .iter()
                .find(|overflow| overflow.group == group.group)
                .cloned();

            if dragged_group == Some(group.group) {
                root = root.child(DockGroupView::render_placeholder(
                    *group,
                    origin,
                    metrics,
                    theme,
                ));
            } else {
                root = root.child(DockGroupView::render_core(
                    None,
                    *group,
                    strip,
                    &headers,
                    overflow,
                    presentation,
                    origin,
                    metrics,
                    0.0,
                    controller.clone(),
                    theme,
                    context_menu.clone(),
                ));
            }
        }

        // X splits and splitters outside Columns are painted once above the
        // child surfaces. Column-owned Y splitters are rendered with their Groups.
        for splitter in splitters.iter().filter(|splitter| {
            !(splitter.axis == dock_core::Axis::Y && column_groups.contains_key(&splitter.owner))
        }) {
            root = root.child(SplitterView::render(
                *splitter,
                origin,
                controller.clone(),
                metrics,
                theme,
                Some(floating.group),
                active_splitter
                    == Some((splitter.owner, splitter.boundary_index, splitter.axis)),
            ));
        }

        // Active panel views outside Columns (standalone Tabs/Panel roots and
        // split children) are hosted directly in the floating composition.
        let loose_panels = panels
            .into_iter()
            .filter_map(|(panel, view)| {
                if column_active_panels.contains(&panel.panel)
                    || dragged_group_panels.contains(&panel.panel)
                {
                    return None;
                }

                Some((panel, view))
            })
            .collect();
        root = root.child(PanelHost::render(loose_panels, origin, theme));

        if !hide_header {
            let close_group = floating.group;
            let manager_for_close = manager.clone();
            let close_action = Box::new(move |app: &mut App| {
                if let Some(manager) = manager_for_close.upgrade() {
                    let _ = manager.update(app, |manager, manager_cx| {
                        if let Err(error) = manager.close_window_grid(close_group, manager_cx) {
                            tracing::warn!(%error, ?close_group, "could not close the Floating WindowGrid");
                        }
                    });
                }
            });
            root = root.child(CompositionHeader::render(
                format!("floating-composition-{:?}", floating.group),
                floating.rect,
                columns,
                metrics,
                theme,
                true,
                Some(close_action),
            ));
        }

        root
    }
}
