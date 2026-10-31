use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockCommand, DockMetrics, DragHandle, DragPayload,
    DragPreview, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, FlyoutState, GroupRect,
    HitResult, NodeId, PanelId, PanelRect, Point, ResizeEdge, SplitterRect, TabHeaderRect,
    TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Context, Entity, IntoElement, MouseButton, ParentElement, Render, ScrollHandle,
    Styled, Window, div, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::{
    DockController,
    input::{
        begin_drag, begin_resize_hit, dispatch_command, finish_drag, finish_resize, update_drag,
        update_resize,
    },
};

use super::{
    column::ColumnView,
    context_menu::GroupMenuToggle,
    group::GroupView,
    panel_host::PanelHost,
    splitter::SplitterView,
    tab_overflow::TabOverflowToggle,
    tab_strip::GroupPresentation,
};

struct FloatingResizeDragGhost;

impl Render for FloatingResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// A floating composition rendered inside the DockHost's workspace.
///
/// The view receives only the Core geometry and the IDs reachable from this
/// floating group's root. It does not create an OS window or keep a second tree.
pub struct FloatingWindowView;

impl FloatingWindowView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        floating: FloatingGroupRect,
        resize_edges: &[FloatingResizeEdgeRect],
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: Vec<(PanelRect, AnyView)>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        rail_display_orders: &HashMap<NodeId, Vec<ColumnDisplayItem>>,
        splitters: &[SplitterRect],
        vertical_overflow: &[VerticalOverflow],
        scroll_handles: &HashMap<NodeId, ScrollHandle>,
        active_flyout: Option<FlyoutState>,
        drag_preview: Option<DragPreview>,
        drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        title: &str,
        menu_group: Option<NodeId>,
        toggle_group_menu: GroupMenuToggle,
        overflow_group: Option<NodeId>,
        toggle_tab_overflow: TabOverflowToggle,
        middle_click_closes_tabs: bool,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let origin = floating.rect.origin;
        let mut column_group_ids = HashSet::new();
        for ids in column_groups.values() {
            column_group_ids.extend(ids.iter().copied());
        }

        let column_active_panels = column_group_ids
            .iter()
            .filter_map(|group| {
                group_presentations
                    .get(group)
                    .and_then(|presentation| presentation.active_panel)
            })
            .collect::<HashSet<PanelId>>();

        let mut root = div()
            .id(format!("floating-window-{:?}", floating.group))
            .absolute()
            .left(px(floating.rect.origin.x))
            .top(px(floating.rect.origin.y))
            .w(px(floating.rect.size.width))
            .h(px(floating.rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, {
                let controller = controller.clone();
                move |_, _, app| {
                    dispatch_command(
                        &controller,
                        DockCommand::RaiseFloating {
                            group: floating.group,
                        },
                        app,
                    );
                }
            });

        // Each Column remains a normal Core subtree. Its groups, tab strips,
        // panel views and internal vertical splitters use the same components
        // as the main dock, only translated into this window's local space.
        for column in columns {
            let group_ids = column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let column_group_rects = groups
                .iter()
                .filter(|group| group_ids.contains(&group.group))
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
            let column_active = group_ids
                .iter()
                .filter_map(|group| {
                    group_presentations
                        .get(group)
                        .and_then(|presentation| presentation.active_panel)
                })
                .collect::<HashSet<_>>();
            let column_panels = panels
                .iter()
                .filter(|(panel, _)| column_active.contains(&panel.panel))
                .cloned()
                .collect::<Vec<_>>();
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);

            root = root.child(ColumnView::render(
                *column,
                mode,
                &column_group_rects,
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
                toggle_group_menu.clone(),
                overflow_group,
                toggle_tab_overflow.clone(),
                middle_click_closes_tabs,
                &column_splitters,
                overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                origin,
                Some(floating.group),
            ));
        }

        // Tabs nodes that are not owned by a Column still use GroupView.
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

            root = root.child(GroupView::render(
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
                Some(floating.group),
                menu_group == Some(group.group),
                toggle_group_menu.clone(),
                overflow_group == Some(group.group),
                toggle_tab_overflow.clone(),
                middle_click_closes_tabs,
            ));
        }

        // X splits and splitters outside Columns are painted once above the
        // child surfaces. Column-owned Y splitters were already rendered by
        // ColumnView, where their scroll offset is available.
        for splitter in splitters.iter().filter(|splitter| {
            !(splitter.axis == dock_core::Axis::Y
                && column_groups.contains_key(&splitter.owner))
        }) {
            root = root.child(SplitterView::render(
                *splitter,
                origin,
                Point::ZERO,
                controller.clone(),
                metrics,
                theme,
                Some(floating.group),
            ));
        }

        // Active panel views outside Columns (standalone Tabs/Panel roots and
        // split children) are hosted directly in the floating composition.
        let loose_panels = panels
            .into_iter()
            .filter_map(|(mut panel, view)| {
                if column_active_panels.contains(&panel.panel) {
                    return None;
                }

                // A standalone Panel has no Tabs header to reserve the title
                // band. Keep its content below the floating title bar.
                let title_bottom = origin.y + metrics.header_height.min(floating.rect.size.height);
                if panel.rect.origin.y < title_bottom {
                    let clipped = (title_bottom - panel.rect.origin.y)
                        .min(panel.rect.size.height);
                    panel.rect.origin.y += clipped;
                    panel.rect.size.height -= clipped;
                    if panel.rect.size.height <= 0.0 {
                        return None;
                    }
                }

                Some((panel, view))
            })
            .collect();
        root = root.child(PanelHost::render(loose_panels, origin));

        // For expanded roots this overlays the first group's existing header;
        // Core reserves the same band for a collapsed composite root. The
        // active tab caption therefore stays in a single, predictable title row.
        let title_height = metrics.header_height.min(floating.rect.size.height);
        let close_size = metrics.group_control_button_width.min(title_height);
        let title_grip = div()
            .id(format!("floating-title-{:?}", floating.group))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .px(px(8.0))
            .overflow_hidden()
            .cursor_move()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title.to_owned()),
            )
            .on_mouse_down(MouseButton::Left, {
                let controller = controller.clone();
                move |event, _, app| {
                    dispatch_command(
                        &controller,
                        DockCommand::RaiseFloating {
                            group: floating.group,
                        },
                        app,
                    );
                    begin_drag(
                        &controller,
                        DragPayload::FloatingGroup {
                            group: floating.group,
                        },
                        DragHandle::FloatingTitle,
                        event,
                        app,
                    );
                    app.stop_propagation();
                }
            })
            .on_mouse_move({
                let controller = controller.clone();
                move |event, _, app| update_drag(&controller, event, app)
            })
            .on_mouse_up({
                let controller = controller.clone();
                move |event, _, app| finish_drag(&controller, event, app)
            })
            .on_mouse_up_out(MouseButton::Left, {
                let controller = controller.clone();
                move |event, _, app| finish_drag(&controller, event, app)
            });

        let close_button = div()
            .id(format!("floating-close-{:?}", floating.group))
            .w(px(close_size.max(1.0)))
            .h(px(close_size.max(1.0)))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(18.0))
            .child("×")
            .on_click({
                let controller = controller.clone();
                move |_, _, app| {
                    dispatch_command(
                        &controller,
                        DockCommand::CloseFloatingGroup {
                            group: floating.group,
                        },
                        app,
                    );
                }
            });

        let title_bar = div()
            .id(format!("floating-title-bar-{:?}", floating.group))
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(title_height))
            .flex()
            .items_center()
            .gap(px(theme.metrics.button_gap))
            .pr(px(4.0))
            .bg(rgb(theme.colors.flyout_header_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(title_grip)
            .child(close_button);
        root = root.child(title_bar);

        // Core emits the eight authoritative hit rectangles. The adapter only
        // translates them into the floating window's local coordinate space.
        for resize in resize_edges.iter().filter(|edge| edge.group == floating.group) {
            let hit = HitResult::FloatingResizeEdge {
                group: floating.group,
                edge: resize.edge,
            };
            let mut handle = div()
                .id(format!("floating-resize-{:?}-{:?}", floating.group, resize.edge))
                .absolute()
                .left(px(resize.rect.origin.x - origin.x))
                .top(px(resize.rect.origin.y - origin.y))
                .w(px(resize.rect.size.width))
                .h(px(resize.rect.size.height))
                .on_mouse_down(MouseButton::Left, {
                    let controller = controller.clone();
                    move |event, _, app| {
                        dispatch_command(
                            &controller,
                            DockCommand::RaiseFloating {
                                group: floating.group,
                            },
                            app,
                        );
                        begin_resize_hit(&controller, hit, event_point(event.position), app);
                        app.stop_propagation();
                    }
                })
                .on_drag(FloatingResizeDrag, |_, _, _, app| {
                    app.new(|_| FloatingResizeDragGhost)
                })
                .on_drag_move::<FloatingResizeDrag>({
                    let controller = controller.clone();
                    move |event, _, app| {
                        update_resize(
                            &controller,
                            event_point(event.event.position),
                            app,
                        );
                    }
                })
                .on_mouse_up(MouseButton::Left, {
                    let controller = controller.clone();
                    move |event, _, app| {
                        finish_resize(
                            &controller,
                            event_point(event.position),
                            event.click_count as usize,
                            app,
                        );
                    }
                })
                .on_mouse_up_out(MouseButton::Left, {
                    let controller = controller.clone();
                    move |event, _, app| {
                        finish_resize(
                            &controller,
                            event_point(event.position),
                            event.click_count as usize,
                            app,
                        );
                    }
                });

            handle = match resize.edge {
                ResizeEdge::Left | ResizeEdge::Right => handle.cursor_col_resize(),
                ResizeEdge::Top | ResizeEdge::Bottom => handle.cursor_row_resize(),
                ResizeEdge::TopLeft | ResizeEdge::BottomRight => handle.cursor_nwse_resize(),
                ResizeEdge::TopRight | ResizeEdge::BottomLeft => handle.cursor_nesw_resize(),
            };
            root = root.child(handle);
        }

        root
    }
}

fn event_point(point: gpui::Point<gpui::Pixels>) -> Point {
    Point::new(point.x.as_f32(), point.y.as_f32())
}
