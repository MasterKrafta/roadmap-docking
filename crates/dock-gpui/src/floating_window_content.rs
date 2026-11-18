use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DragPreview, DropZoneRect,
    FloatingGroupRect, FlyoutState, GroupRect, NodeId, PanelRect, Point, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, App, Entity, IntoElement, ParentElement, Styled, WeakEntity, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use super::floating_column_host_rect;

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

/// Shared Floating presentation helpers.
///
/// `render` remains the legacy whole-composition path. `render_column` is used
/// by permanent Floating Column hosts and shares the Main Column/Group renderers.
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

        for (index, column) in columns.iter().copied().enumerate() {
            let show_close_button = !hide_header && index + 1 == columns.len();
            let close_action = show_close_button.then(|| {
                let close_group = floating.group;
                let manager_for_close = manager.clone();
                Box::new(move |app: &mut App| {
                    if let Some(manager) = manager_for_close.upgrade() {
                        let _ = manager.update(app, |manager, manager_cx| {
                            if let Err(error) = manager.close_window_grid(close_group, manager_cx) {
                                tracing::warn!(%error, ?close_group, "could not close the Floating WindowGrid");
                            }
                        });
                    }
                }) as Box<dyn Fn(&mut App) + 'static>
            });

            root = root.child(Self::render_column(
                floating,
                column,
                Point::ZERO,
                groups,
                &panels,
                dragged_group,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                column_groups,
                column_modes,
                rail_display_orders,
                splitters,
                active_splitter,
                vertical_overflow,
                scroll_handles,
                active_flyout,
                metrics,
                !hide_header,
                show_close_button,
                close_action,
                false,
                menu_group,
                context_menu.clone(),
                controller.clone(),
                theme,
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

        root
    }

    /// Renders one complete Column presentation inside the caller's viewport.
    ///
    /// Core rectangles remain in composition-local coordinates. parent_origin
    /// translates that shared geometry into the local coordinate space of the
    /// native Column window without duplicating or mutating the geometry snapshot.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_column(
        floating: FloatingGroupRect,
        column: ColumnRect,
        parent_origin: Point,
        groups: &[GroupRect],
        panels: &[(PanelRect, Option<AnyView>)],
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
        metrics: DockMetrics,
        show_header: bool,
        show_close_button: bool,
        close_action: Option<Box<dyn Fn(&mut App) + 'static>>,
        render_x_splitters: bool,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
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

        let mut root = div()
            .id(format!("floating-column-{:?}-{:?}", floating.group, column.column))
            .absolute()
            .left(px(-parent_origin.x))
            .top(px(-parent_origin.y))
            .size_full()
            .child(render_column_content(
                column,
                group_ids,
                groups,
                panels,
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
                context_menu,
                splitters,
                active_splitter,
                overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                Some(floating.group),
                show_header,
                dragged_group,
            ));

        if show_header {
            let header_rect = dock_core::Rect::new(
                column.rect.origin,
                dock_core::Size::new(column.rect.size.width, metrics.column_header_height),
            );
            root = root.child(CompositionHeader::render(
                format!("floating-column-header-{:?}", column.column),
                header_rect,
                std::slice::from_ref(&column),
                metrics,
                theme,
                show_close_button,
                close_action,
            ));
        }

        if render_x_splitters {
            let host_rect = floating_column_host_rect(
                column.rect,
                splitters,
                metrics.splitter_thickness,
            );
            let host_right = host_rect.origin.x + host_rect.size.width;
            let host_bottom = host_rect.origin.y + host_rect.size.height;

            for splitter in splitters.iter().filter(|splitter| {
                if splitter.axis != dock_core::Axis::X {
                    return false;
                }
                let splitter_right = splitter.rect.origin.x + splitter.rect.size.width;
                let splitter_bottom = splitter.rect.origin.y + splitter.rect.size.height;
                splitter.rect.origin.x < host_right
                    && splitter_right > host_rect.origin.x
                    && splitter.rect.origin.y < host_bottom
                    && splitter_bottom > host_rect.origin.y
            }) {
                root = root.child(SplitterView::render(
                    *splitter,
                    Point::ZERO,
                    controller.clone(),
                    metrics,
                    theme,
                    Some(floating.group),
                    active_splitter
                        == Some((splitter.owner, splitter.boundary_index, splitter.axis)),
                ));
            }
        }

        root
    }
}
