use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use dock_core::{
    ColumnRect, DockSide, DragPayload, DragPreview,
    DropZoneRect, FloatingGroupId, GroupRect, Node, NodeId, PanelRect, Point, Rect,
    TabHeaderRect, TabStripRect,
};
use gpui::{
    App, Context, Entity, FocusHandle, HitboxId, IntoElement, ParentElement, Render, ScrollHandle,
    Styled, WeakEntity, Window, div, prelude::InteractiveElement as _, rgb,
};
use ui::Theme;

use crate::{
    components::{main_dock::render_column_content, DropOverlay},
    dock_components::{
        active_panels_for_groups, collect_group_presentations, CompositionHeader, DockGroupView,
        PanelTransferPresentation,
    },
    dock_host::{build_context_menu_entries, context_menu_target_exists},
    DockController, DockWindowPart, FloatingWindowManager,
};

use super::{
    floating_dock_component_tab_zones, floating_host_snapshot, floating_part_local_rect,
    floating_part_owner_and_column, handle_floating_escape, observe_floating_focus_loss,
    render_floating_pointer_input, FloatingHostOpenMenu, FloatingHostSnapshot, FloatingPartKind,
    FloatingTabPress, FloatingGroupPress, FloatingWindowContent,
};
use super::super::{
    context_menu::{
        ContextMenu, ContextMenuCallbacks, ContextMenuDismiss, ContextMenuSubmenuToggle,
        GroupMenuToggle,
    },
    panel_host::PanelHost,
};

/// Root view for one complete Floating Column window.
///
/// Legacy Header/Group render paths remain temporarily available for drag migration;
/// permanent Floating windows are reconciled by Column identity.
pub(crate) struct FloatingPartHost {
    controller: Entity<DockController>,
    floating_group: Option<FloatingGroupId>,
    part: FloatingPartKind,
    main_drag_side: Option<DockSide>,
    theme: Theme,
    manager: WeakEntity<FloatingWindowManager>,
    controller_observed: bool,
    manager_observed: bool,
    show_close_button: bool,
    open_context_menu: Option<FloatingHostOpenMenu>,
    tab_press: Rc<Cell<Option<FloatingTabPress>>>,
    group_press: Rc<Cell<Option<FloatingGroupPress>>>,
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
    column_scroll_handles: HashMap<NodeId, ScrollHandle>,
    root_focus_handle: Option<FocusHandle>,
    focus_observed: bool,
    native_bounds_observed: bool,
    native_resize_observation_pending: bool,
}

impl FloatingPartHost {
    pub(crate) fn set_show_close_button(&mut self, show: bool) -> bool {
        if self.show_close_button == show {
            return false;
        }
        self.show_close_button = show;
        true
    }

    pub(crate) fn new_column(
        controller: Entity<DockController>,
        floating_group: FloatingGroupId,
        column: NodeId,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self::new(
            controller,
            Some(floating_group),
            FloatingPartKind::Column { column },
            theme,
            manager,
        )
    }

    pub(crate) fn new_header(
        controller: Entity<DockController>,
        floating_group: FloatingGroupId,
        column: NodeId,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self::new(
            controller,
            Some(floating_group),
            FloatingPartKind::Header { column },
            theme,
            manager,
        )
    }

    pub(crate) fn new_group(
        controller: Entity<DockController>,
        floating_group: FloatingGroupId,
        column: NodeId,
        group: NodeId,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self::new(
            controller,
            Some(floating_group),
            FloatingPartKind::Group { column, group },
            theme,
            manager,
        )
    }

    pub(crate) fn new_main_drag(
        controller: Entity<DockController>,
        side: DockSide,
        part: FloatingPartKind,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        let mut host = Self::new(
            controller,
            None,
            part,
            theme,
            manager,
        );
        host.main_drag_side = Some(side);
        host
    }

    pub(crate) fn adopt_main_drag_part(&mut self, floating_group: FloatingGroupId) -> bool {
        if self.main_drag_side.take().is_none() {
            return false;
        }
        self.floating_group = Some(floating_group);
        true
    }

    fn new(
        controller: Entity<DockController>,
        floating_group: Option<FloatingGroupId>,
        part: FloatingPartKind,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self {
            controller,
            floating_group,
            part,
            main_drag_side: None,
            theme,
            manager,
            controller_observed: false,
            manager_observed: false,
            show_close_button: false,
            open_context_menu: None,
            tab_press: Rc::new(Cell::new(None)),
            group_press: Rc::new(Cell::new(None)),
            pointer_capture: Rc::new(Cell::new(None)),
            column_scroll_handles: HashMap::new(),
            root_focus_handle: None,
            focus_observed: false,
            native_bounds_observed: false,
            native_resize_observation_pending: false,
        }
    }
}

impl Render for FloatingPartHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        if let Some(side) = self.main_drag_side {
            return self.render_main_drag_part(window, cx, side);
        }

        // A persistent Column HWND or preserved Group HWND can survive a Core
        // ownership transfer. Resolve the current FloatingGroup and Column from Layout
        // so geometry, hit-testing and native resize callbacks follow the unchanged HWND.
        if let Some((current_group, current_column)) = self.controller.update(cx, |controller, _| {
            floating_part_owner_and_column(controller.layout(), self.part)
        }) {
            self.floating_group = Some(current_group);
            self.part = match self.part {
                FloatingPartKind::Column { .. } => FloatingPartKind::Column {
                    column: current_column,
                },
                FloatingPartKind::Group { group, .. } => FloatingPartKind::Group {
                    column: current_column,
                    group,
                },
                FloatingPartKind::Header { .. } => FloatingPartKind::Header {
                    column: current_column,
                },
            };
        }

        let window_id = window.window_handle().window_id();

        if let (Some(group), FloatingPartKind::Column { column }) =
            (self.floating_group, self.part)
        {
            if !self.native_bounds_observed {
                cx.observe_window_bounds(window, |host, window, host_cx| {
                    let (Some(group), FloatingPartKind::Column { column }) =
                        (host.floating_group, host.part)
                    else {
                        return;
                    };
                    let window_id = window.window_handle().window_id();

                    // FloatingWindowManager can call bounds_changed() while it
                    // is reconciling this HWND. Defer manager updates to avoid
                    // re-entering the manager entity from GPUI's observer.
                    host_cx.defer_in(window, move |host, _, deferred_cx| {
                        if host.floating_group != Some(group)
                            || !matches!(
                                host.part,
                                FloatingPartKind::Column { column: current } if current == column
                            )
                        {
                            return;
                        }
                        if let Some(manager) = host.manager.upgrade() {
                            let _ = manager.update(deferred_cx, |manager, manager_cx| {
                                manager.sync_native_column_resize_bounds(
                                    column,
                                    group,
                                    window_id,
                                    manager_cx,
                                );
                            });
                        }
                    });
                })
                .detach();
                self.native_bounds_observed = true;
            }

            if !self.native_resize_observation_pending {
                self.native_resize_observation_pending = true;
                cx.defer_in(window, move |host, _, host_cx| {
                    host.native_resize_observation_pending = false;
                    if host.floating_group == Some(group)
                        && matches!(
                            host.part,
                            FloatingPartKind::Column { column: current } if current == column
                        )
                        && let Some(manager) = host.manager.upgrade()
                    {
                        manager.update(host_cx, |manager, manager_cx| {
                            manager.observe_native_column_resize(
                                column,
                                group,
                                window_id,
                                manager_cx,
                            );
                        });
                    }
                });
            }
        }

        let focus_handle = self
            .root_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !self.focus_observed {
            observe_floating_focus_loss(
                cx,
                &focus_handle,
                window,
                self.controller.clone(),
                self.manager.clone(),
                self.tab_press.clone(),
                self.group_press.clone(),
                self.pointer_capture.clone(),
            );
            self.focus_observed = true;
        }
        if !self.manager_observed {
            if let Some(manager) = self.manager.upgrade() {
                cx.observe(&manager, |_, _, host_cx| host_cx.notify()).detach();
            }
            self.manager_observed = true;
        }
        if !matches!(self.part, FloatingPartKind::Header { .. }) {
            self.controller.update(cx, |controller, panel_cx| {
                controller.ensure_panel_views(window, panel_cx);
            });
        }

        let floating_group = self.floating_group();
        let column_viewport = match self.part {
            FloatingPartKind::Column { column } => Some((
                column,
                f32::from(window.bounds().size.height),
            )),
            _ => None,
        };
        let snapshot = self.controller.update(cx, |controller, _| {
            if let Some((column, height)) = column_viewport {
                let origin_y_offset = controller
                    .floating_column_viewport_origin_y_offset(
                        window_id,
                        floating_group,
                        column,
                    )
                    .unwrap_or(0.0);
                controller.set_floating_column_viewport(
                    window_id,
                    floating_group,
                    column,
                    height,
                    origin_y_offset,
                );
            }
            floating_host_snapshot(controller, window_id, floating_group)
        });
        let snapshot = match snapshot {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return div().id("empty-floating-part").size_full().into_any_element(),
            Err(error) => {
                tracing::warn!(
                    ?self.part,
                    %error,
                    "floating part host could not obtain Core geometry"
                );
                return div().id("invalid-floating-part").size_full().into_any_element();
            }
        };

        match self.part {
            FloatingPartKind::Column { column } => div()
                .id(format!("floating-column-part-window-root-{:?}", column))
                .track_focus(&focus_handle)
                .relative()
                .size_full()
                .overflow_hidden()
                .bg(rgb(self.theme.colors.panel_surface_background))
                .child(self.render_floating_column(window, cx, &snapshot, column))
                .into_any_element(),
            FloatingPartKind::Header { column } => {
                let root = div()
                    .id(format!("floating-header-host-root-{:?}", column))
                    .track_focus(&focus_handle)
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .bg(rgb(self.theme.colors.panel_surface_background));
                let Some(column_rect) = snapshot
                    .rects
                    .columns
                    .iter()
                    .find(|candidate| candidate.column == column)
                else {
                    return root.into_any_element();
                };
                let header_rect = Rect::new(
                    Point::ZERO,
                    dock_core::Size::new(
                        column_rect.rect.size.width,
                        snapshot.metrics.column_header_height,
                    ),
                );
                let local_column = ColumnRect::new(column, header_rect);
                let show_close_button = self.show_close_button;
                let pointer_header_rect = Rect::new(
                    column_rect.rect.origin,
                    dock_core::Size::new(
                        column_rect.rect.size.width,
                        snapshot.metrics.column_header_height,
                    ),
                );
                let pointer_column_rect = ColumnRect::new(column, pointer_header_rect);
                let pointer_input = render_floating_pointer_input(
                    self.floating_group(),
                    self.manager.clone(),
                    self.controller.clone(),
                    window.window_handle().window_id(),
                    Some(snapshot.floating.rect),
                    HashMap::new(),
                    Some((
                        pointer_header_rect,
                        vec![pointer_column_rect],
                        Vec::new(),
                        snapshot.metrics,
                        show_close_button,
                    )),
                    false,
                    self.tab_press.clone(),
                    self.group_press.clone(),
                    self.pointer_capture.clone(),
                    Some(DockWindowPart::Header { column }),
                    column_rect.rect.origin,
                );
                let close_group = self.floating_group();
                let manager_for_close = self.manager.clone();
                let close_action = Box::new(move |app: &mut App| {
                    if let Some(manager) = manager_for_close.upgrade() {
                        let _ = manager.update(app, |manager, manager_cx| {
                            if let Err(error) = manager.close_window_grid(close_group, manager_cx) {
                                tracing::warn!(%error, ?close_group, "could not close the Floating WindowGrid");
                            }
                        });
                    }
                });
                let mut root = root
                    .child(pointer_input)
                    .child(CompositionHeader::render(
                        format!("floating-column-header-{:?}", column),
                        header_rect,
                        &[local_column],
                        snapshot.metrics,
                        self.theme,
                        show_close_button,
                        Some(close_action),
                    ));
                if snapshot.drag_preview.is_some() {
                    root = root.child(render_floating_drop_overlay(
                        &snapshot,
                        column_rect.rect.origin,
                        self.theme,
                    ));
                }
                let manager_for_escape = self.manager.clone();
                let controller_for_escape = self.controller.clone();
                let tab_press_for_escape = self.tab_press.clone();
                let group_press_for_escape = self.group_press.clone();
                let pointer_capture_for_escape = self.pointer_capture.clone();
                root = root.capture_key_down(move |event, window, app| {
                    handle_floating_escape(
                        event,
                        false,
                        &manager_for_escape,
                        &controller_for_escape,
                        &tab_press_for_escape,
                        &group_press_for_escape,
                        &pointer_capture_for_escape,
                        window,
                        app,
                    );
                });
                root.into_any_element()
            }
            FloatingPartKind::Group { column, group } => div()
                .id(format!("floating-group-part-window-root-{:?}", group))
                .track_focus(&focus_handle)
                .relative()
                .size_full()
                .overflow_hidden()
                .bg(rgb(self.theme.colors.panel_surface_background))
                .child(self.render_group(window, cx, &snapshot, column, group))
                .into_any_element(),
        }
    }
}

fn render_floating_drop_overlay(
    snapshot: &FloatingHostSnapshot,
    origin: Point,
    theme: Theme,
) -> impl IntoElement + use<> {
    let groups = snapshot
        .rects
        .groups
        .iter()
        .map(|group| {
            GroupRect::new(
                group.group,
                floating_part_local_rect(group.rect, origin),
            )
        })
        .collect::<Vec<_>>();
    let columns = snapshot
        .rects
        .columns
        .iter()
        .map(|column| {
            ColumnRect::new(
                column.column,
                floating_part_local_rect(column.rect, origin),
            )
        })
        .collect::<Vec<_>>();
    let tab_headers = snapshot
        .rects
        .tab_headers
        .iter()
        .map(|header| {
            TabHeaderRect::new(
                header.group,
                header.panel,
                header.index,
                floating_part_local_rect(header.rect, origin),
            )
        })
        .collect::<Vec<_>>();
    let tab_strips = snapshot
        .rects
        .tab_strips
        .iter()
        .map(|strip| {
            TabStripRect::new(
                strip.group,
                floating_part_local_rect(strip.rect, origin),
            )
        })
        .collect::<Vec<_>>();
    let resolved_zone = snapshot.active_drop_zone.map(|zone| {
        DropZoneRect::new(
            zone.target,
            zone.zone,
            floating_part_local_rect(zone.rect, origin),
        )
    });
    let preview = snapshot.drag_preview.map(|preview| DragPreview {
        point: Point::new(
            preview.point.x - origin.x,
            preview.point.y - origin.y,
        ),
        ..preview
    });
    let floating_bounds = floating_part_local_rect(snapshot.floating.rect, origin);

    DropOverlay::render(
        resolved_zone,
        preview,
        &groups,
        &columns,
        &snapshot.column_groups,
        &tab_headers,
        &tab_strips,
        None,
        Some(floating_bounds),
        theme,
    )
}

impl FloatingPartHost {
    fn render_main_drag_part(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        side: DockSide,
    ) -> gpui::AnyElement {
        match self.part {
            FloatingPartKind::Column { column } => {
                self.render_main_drag_column(cx, side, column).into_any_element()
            }
            FloatingPartKind::Header { column } => {
                let header = self.controller.update(cx, |controller, _| {
                    let session = controller.drag().host_session()?;
                    if !matches!(
                        session.payload(),
                        DragPayload::DockArea { side: active } if active == side
                    ) || !session.all_panel_views_source_detached()
                    {
                        return None;
                    }
                    let rects = controller.geometry().ok()?;
                    let column_rect = rects.columns.iter()
                        .find(|rect| rect.column == column)?;
                    Some((
                        column_rect.rect.size.width,
                        controller.metrics(),
                    ))
                });

                let Some((width, metrics)) = header else {
                    return div().id(format!("main-drag-header-waiting-{:?}", column)).size_full().into_any_element();
                };
                let local_header = ColumnRect::new(
                    column,
                    Rect::new(
                        Point::ZERO,
                        dock_core::Size::new(width, metrics.column_header_height),
                    ),
                );
                div()
                    .id(format!("main-drag-header-{:?}", column))
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .bg(rgb(self.theme.colors.panel_surface_background))
                    .child(CompositionHeader::render(
                        format!("main-drag-column-header-{:?}", column),
                        Rect::new(
                            Point::ZERO,
                            dock_core::Size::new(width, metrics.column_header_height),
                        ),
                        &[local_header],
                        metrics,
                        self.theme,
                        false,
                        None,
                    ))
                    .into_any_element()
            }
            FloatingPartKind::Group { column, group } => {
                let snapshot = self.controller.update(cx, |controller, _| {
                    let session = controller.drag().host_session()?;
                    if !matches!(
                        session.payload(),
                        DragPayload::DockArea { side: active } if active == side
                    ) || !session.all_panel_views_source_detached()
                    {
                        return None;
                    }

                    let rects = controller.geometry().ok()?.clone();
                    let group_rect = rects.groups.iter()
                        .find(|rect| rect.group == group).copied()?;
                    let strip = rects.tab_strips.iter()
                        .find(|strip| strip.group == group).copied()?;
                    let headers = rects.tab_headers.iter()
                        .filter(|header| header.group == group)
                        .copied()
                        .collect::<Vec<_>>();
                    let overflow = rects.tab_overflows.iter()
                        .find(|overflow| overflow.group == group)
                        .cloned();
                    let presentations = collect_group_presentations(
                        controller.layout(),
                        controller.registry(),
                        &HashSet::from([group]),
                        Some(&rects),
                    );
                    let presentation = presentations.get(&group)?.clone();
                    let panel = presentation.active_panel?;
                    let panel_rect = rects.panels.iter()
                        .find(|rect| rect.panel == panel).copied()?;
                    let panel_view = controller.panel_view_for_drag(panel)?;
                    Some((
                        group_rect,
                        strip,
                        headers,
                        overflow,
                        presentation,
                        panel_rect,
                        panel_view,
                        controller.metrics(),
                    ))
                });

                let Some((
                    group_rect,
                    strip,
                    headers,
                    overflow,
                    presentation,
                    panel_rect,
                    panel_view,
                    metrics,
                )) = snapshot else {
                    return div().id(format!("main-drag-group-waiting-{:?}", group)).size_full().into_any_element();
                };

                let local_group = GroupRect::new(
                    group,
                    Rect::new(Point::ZERO, group_rect.rect.size),
                );
                let local_strip = TabStripRect::new(
                    group,
                    floating_part_local_rect(strip.rect, group_rect.rect.origin),
                );
                let local_headers = headers.iter().map(|header| {
                    TabHeaderRect::new(
                        group,
                        header.panel,
                        header.index,
                        floating_part_local_rect(header.rect, group_rect.rect.origin),
                    )
                }).collect::<Vec<_>>();
                let local_overflow = overflow.map(|mut value| {
                    value.button_rect = floating_part_local_rect(
                        value.button_rect,
                        group_rect.rect.origin,
                    );
                    value
                });
                let local_panel = PanelRect::new(
                    panel_rect.panel,
                    floating_part_local_rect(panel_rect.rect, group_rect.rect.origin),
                );
                let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});

                div()
                    .id(format!("main-drag-group-{:?}", group))
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .bg(rgb(self.theme.colors.panel_surface_background))
                    .child(DockGroupView::render_core(
                        Some(column),
                        local_group,
                        local_strip,
                        &local_headers,
                        local_overflow,
                        &presentation,
                        Point::ZERO,
                        metrics,
                        0.0,
                        self.controller.clone(),
                        self.theme,
                        toggle_menu,
                    ))
                    .child(PanelHost::render(
                        vec![(local_panel, Some(panel_view))],
                        Point::ZERO,
                        self.theme,
                    ))
                    .into_any_element()
            }
        }
    }

    fn render_main_drag_column(
        &mut self,
        cx: &mut Context<Self>,
        side: DockSide,
        column_id: NodeId,
    ) -> impl IntoElement {
        let snapshot = self.controller.update(cx, |controller, _| {
            let session = controller.drag().host_session()?;
            if !matches!(
                session.payload(),
                DragPayload::DockArea { side: active } if active == side
            ) || !session.all_panel_views_source_detached()
            {
                return None;
            }

            let rects = controller.geometry().ok()?.clone();
            let column_rect = rects
                .columns
                .iter()
                .find(|rect| rect.column == column_id)
                .copied()?;
            let Node::Column(column) = controller.layout().node(column_id)? else {
                return None;
            };
            let group_ids = column.groups.clone();
            let group_id_set = group_ids.iter().copied().collect::<HashSet<_>>();
            let group_presentations = collect_group_presentations(
                controller.layout(),
                controller.registry(),
                &group_id_set,
                Some(&rects),
            );
            let active_panels =
                active_panels_for_groups(group_ids.iter().copied(), &group_presentations, None);
            let panels = rects
                .panels
                .iter()
                .filter(|panel| active_panels.contains(&panel.panel))
                .filter_map(|panel| {
                    controller
                        .panel_view_for_drag(panel.panel)
                        .map(|view| (*panel, Some(view)))
                })
                .collect::<Vec<_>>();
            let groups = rects
                .groups
                .iter()
                .filter(|group| group_id_set.contains(&group.group))
                .copied()
                .collect::<Vec<_>>();
            let tab_strips = rects
                .tab_strips
                .iter()
                .filter(|strip| group_id_set.contains(&strip.group))
                .copied()
                .collect::<Vec<_>>();
            let tab_headers = rects
                .tab_headers
                .iter()
                .filter(|header| group_id_set.contains(&header.group))
                .copied()
                .collect::<Vec<_>>();
            let tab_overflows = rects
                .tab_overflows
                .iter()
                .filter(|overflow| group_id_set.contains(&overflow.group))
                .cloned()
                .collect::<Vec<_>>();
            let rail_display_order = if column.mode.is_rail() {
                controller.layout().column_display_order(column_id).ok()
            } else {
                None
            };
            Some((
                column_rect,
                groups,
                panels,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                group_ids,
                column.mode,
                rail_display_order.unwrap_or_default(),
                rects.splitters,
                rects.vertical_overflow,
                controller.metrics(),
            ))
        });

        let Some((
            column_rect,
            groups,
            panels,
            tab_strips,
            tab_headers,
            tab_overflows,
            group_presentations,
            group_ids,
            column_mode,
            rail_display_order,
            splitters,
            vertical_overflow,
            metrics,
        )) = snapshot
        else {
            return div()
                .id(format!("main-drag-column-waiting-{:?}", column_id))
                .size_full();
        };

        let local_column = ColumnRect::new(
            column_id,
            Rect::new(Point::ZERO, column_rect.rect.size),
        );
        let has_vertical_overflow = vertical_overflow
            .iter()
            .any(|overflow| overflow.owner == column_id);
        self.column_scroll_handles
            .retain(|column, _| *column == column_id && has_vertical_overflow);
        if has_vertical_overflow {
            self.column_scroll_handles.entry(column_id).or_default();
        }
        let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
        let content = render_column_content(
            column_rect,
            &group_ids,
            &groups,
            &panels,
            column_mode,
            &tab_strips,
            &tab_headers,
            &tab_overflows,
            &group_presentations,
            &rail_display_order,
            None,
            metrics,
            None,
            toggle_menu,
            &splitters,
            None,
            vertical_overflow
                .iter()
                .find(|overflow| overflow.owner == column_id)
                .copied(),
            self.column_scroll_handles.get(&column_id).cloned(),
            self.controller.clone(),
            self.theme,
            None,
            true,
            None,
        );
        let header = CompositionHeader::render(
            format!("main-drag-column-header-{:?}", column_id),
            Rect::new(Point::ZERO, column_rect.rect.size),
            std::slice::from_ref(&local_column),
            metrics,
            self.theme,
            false,
            None,
        );

        div()
            .id(format!("main-drag-column-{:?}", column_id))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background))
            .child(
                div()
                    .absolute()
                    .left(gpui::px(-column_rect.rect.origin.x))
                    .top(gpui::px(-column_rect.rect.origin.y))
                    .size_full()
                    .child(content),
            )
            .child(header)
    }

    fn floating_group(&self) -> FloatingGroupId {
        self.floating_group
            .expect("FloatingPartHost must receive its Core FloatingGroup before permanent rendering")
    }

    fn render_floating_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        snapshot: &FloatingHostSnapshot,
        column_id: NodeId,
    ) -> impl IntoElement {
        let window_id = window.window_handle().window_id();
        let Some(column_rect) = snapshot
            .rects
            .columns
            .iter()
            .find(|candidate| candidate.column == column_id)
            .copied()
        else {
            return div().id("missing-floating-column").size_full();
        };
        let group_ids = snapshot
            .column_groups
            .get(&column_id)
            .cloned()
            .unwrap_or_default();
        let group_id_set = group_ids.iter().copied().collect::<HashSet<_>>();
        let groups = snapshot
            .rects
            .groups
            .iter()
            .filter(|group| group_id_set.contains(&group.group))
            .copied()
            .collect::<Vec<_>>();
        let transferred_panels = self.controller.update(cx, |controller, _| {
            controller.transferred_panel_views_from(window_id)
        });
        let transfer = PanelTransferPresentation::new(
            &snapshot.group_presentations,
            &transferred_panels,
        );
        let mut presentations = snapshot.group_presentations.clone();
        transfer.apply_to_group_presentations(&mut presentations);

        let dragged_group = self.controller.update(cx, |controller, _| {
            controller.dragged_group_from(window_id)
        });
        let tab_strips = snapshot
            .rects
            .tab_strips
            .iter()
            .filter(|strip| {
                group_id_set.contains(&strip.group)
                    && (!transfer.groups_without_visible_tabs.contains(&strip.group)
                        || dragged_group == Some(strip.group))
            })
            .copied()
            .collect::<Vec<_>>();
        let tab_headers = transfer
            .visible_tab_headers(
                &snapshot.rects.tab_headers,
                &snapshot.rects.tab_strips,
                &snapshot.rects.tab_overflows,
                &presentations,
                &group_id_set,
                snapshot.metrics,
            )
            .into_iter()
            .filter(|header| group_id_set.contains(&header.group))
            .collect::<Vec<_>>();
        let tab_overflows = snapshot
            .rects
            .tab_overflows
            .iter()
            .filter(|overflow| {
                group_id_set.contains(&overflow.group)
                    && !transfer.groups_without_visible_tabs.contains(&overflow.group)
            })
            .cloned()
            .collect::<Vec<_>>();
        let active_panels =
            active_panels_for_groups(group_ids.iter().copied(), &presentations, dragged_group);
        let panels = transfer
            .panel_rects_with_active_replacements(&snapshot.rects.panels)
            .into_iter()
            .filter(|panel| {
                active_panels.contains(&panel.panel)
                    && !transfer.transferred_panels.contains(&panel.panel)
                    && !transfer.panels_in_hidden_groups.contains(&panel.panel)
            })
            .map(|panel| {
                let view = self.controller.update(cx, |controller, panel_cx| {
                    controller.panel_view(panel.panel, window, panel_cx)
                });
                (panel, view)
            })
            .collect::<Vec<_>>();
        let pending_transfers = self.controller.update(cx, |controller, _| {
            controller.pending_panel_view_transfers_from(window_id)
        });
        PanelHost::confirm_source_detached_after_render(
            &self.controller,
            window,
            window_id,
            pending_transfers,
            &panels,
        );

        let has_vertical_overflow = snapshot
            .rects
            .vertical_overflow
            .iter()
            .any(|overflow| overflow.owner == column_id);
        self.column_scroll_handles
            .retain(|column, _| *column == column_id && has_vertical_overflow);
        if has_vertical_overflow {
            self.column_scroll_handles.entry(column_id).or_default();
        }

        let show_close_button = self.show_close_button;
        let close_action = if show_close_button {
            let close_group = snapshot.floating.group;
            let manager_for_close = self.manager.clone();
            Some(Box::new(move |app: &mut App| {
                if let Some(manager) = manager_for_close.upgrade() {
                    let _ = manager.update(app, |manager, manager_cx| {
                        if let Err(error) = manager.close_window_grid(close_group, manager_cx) {
                            tracing::warn!(%error, ?close_group, "could not close the Floating WindowGrid");
                        }
                    });
                }
            }) as Box<dyn Fn(&mut App) + 'static>)
        } else {
            None
        };

        let pointer_input = render_floating_pointer_input(
            self.floating_group(),
            self.manager.clone(),
            self.controller.clone(),
            window_id,
            Some(snapshot.floating.rect),
            floating_dock_component_tab_zones(snapshot),
            Some((
                snapshot.floating.rect,
                snapshot.rects.columns.clone(),
                snapshot.rects.splitters.clone(),
                snapshot.metrics,
                show_close_button,
            )),
            false,
            self.tab_press.clone(),
            self.group_press.clone(),
            self.pointer_capture.clone(),
            Some(DockWindowPart::Column { column: column_id }),
            column_rect.rect.origin,
        );

        let rendered_panels = panels
            .iter()
            .filter_map(|(panel, view)| view.as_ref().map(|_| panel.panel))
            .collect::<HashSet<_>>();
        let manager_for_handoff = self.manager.clone();
        let floating_group = snapshot.floating.group;
        window.on_next_frame(move |_, app| {
            let Some(manager) = manager_for_handoff.upgrade() else {
                return;
            };
            let Some(required_panels) = manager.update(app, |manager, manager_cx| {
                manager.pending_drag_host_handoff_panels(floating_group, window_id, manager_cx)
            }) else {
                return;
            };
            if required_panels
                .iter()
                .all(|panel| rendered_panels.contains(panel))
            {
                let _ = manager.update(app, |manager, manager_cx| {
                    manager.notify_drag_host_handoff_rendered(
                        floating_group,
                        window_id,
                        &required_panels,
                        manager_cx,
                    );
                });
            }
        });

        let toggle_host = cx.entity().downgrade();
        let toggle_context_menu: GroupMenuToggle = Rc::new(move |target, anchor, app| {
            if let Some(host) = toggle_host.upgrade() {
                host.update(app, |host, host_cx| {
                    host.open_context_menu = if host
                        .open_context_menu
                        .as_ref()
                        .is_some_and(|open| open.target == target)
                    {
                        None
                    } else {
                        Some(FloatingHostOpenMenu {
                            target,
                            anchor,
                            open_submenu: None,
                        })
                    };
                    host_cx.notify();
                });
            }
        });

        let mut root = div()
            .id(format!("floating-column-content-{:?}", column_id))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background))
            .child(pointer_input)
            .child(FloatingWindowContent::render_column(
                snapshot.floating,
                column_rect,
                column_rect.rect.origin,
                &groups,
                &panels,
                dragged_group,
                &tab_strips,
                &tab_headers,
                &tab_overflows,
                &presentations,
                &snapshot.column_groups,
                &snapshot.column_modes,
                &snapshot.rail_display_orders,
                &snapshot.rects.splitters,
                snapshot.resize_preview.and_then(|preview| preview.splitter),
                &snapshot.rects.vertical_overflow,
                &self.column_scroll_handles,
                None,
                snapshot.metrics,
                true,
                show_close_button,
                close_action,
                None,
                toggle_context_menu,
                self.controller.clone(),
                self.theme,
            ));

        if snapshot.drag_preview.is_some() {
            root = root.child(render_floating_drop_overlay(
                snapshot,
                column_rect.rect.origin,
                self.theme,
            ));
        }

        if let Some(open) = self.open_context_menu.as_ref()
            && context_menu_target_exists(
                open.target,
                &presentations,
                &snapshot.column_ids,
                Some(&snapshot.rects),
            )
        {
            let entries = build_context_menu_entries(
                open.target,
                &snapshot.rects,
                &presentations,
                &snapshot.column_modes,
                self.controller.clone(),
                false,
            );
            if !entries.is_empty() {
                let submenu_host = cx.entity().downgrade();
                let toggle_context_submenu: ContextMenuSubmenuToggle = Rc::new(move |index, app| {
                    if let Some(host) = submenu_host.upgrade() {
                        host.update(app, |host, host_cx| {
                            if let Some(open) = host.open_context_menu.as_mut() {
                                open.open_submenu = index;
                                host_cx.notify();
                            }
                        });
                    }
                });
                let dismiss_host = cx.entity().downgrade();
                let dismiss_context_menu: ContextMenuDismiss = Rc::new(move |app| {
                    if let Some(host) = dismiss_host.upgrade() {
                        host.update(app, |host, host_cx| {
                            host.open_context_menu = None;
                            host_cx.notify();
                        });
                    }
                });
                root = root.child(ContextMenu::render(
                    "floating-column-context-menu",
                    Rect::new(Point::ZERO, column_rect.rect.size),
                    open.anchor,
                    &entries,
                    open.open_submenu,
                    ContextMenuCallbacks {
                        on_submenu: toggle_context_submenu,
                        on_dismiss: dismiss_context_menu,
                    },
                    self.theme,
                ));
            }
        }

        let manager_for_escape = self.manager.clone();
        let controller_for_escape = self.controller.clone();
        let tab_press_for_escape = self.tab_press.clone();
        let group_press_for_escape = self.group_press.clone();
        let pointer_capture_for_escape = self.pointer_capture.clone();
        let context_menu_is_open = self.open_context_menu.is_some();
        root = root.capture_key_down(move |event, window, app| {
            handle_floating_escape(
                event,
                context_menu_is_open,
                &manager_for_escape,
                &controller_for_escape,
                &tab_press_for_escape,
                &group_press_for_escape,
                &pointer_capture_for_escape,
                window,
                app,
            );
        });

        root
    }

    fn render_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        snapshot: &FloatingHostSnapshot,
        column: NodeId,
        group_id: NodeId,
    ) -> impl IntoElement {
        let mut root = div()
            .id(format!("floating-group-host-root-{:?}", group_id))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background));
        if !snapshot
            .column_groups
            .get(&column)
            .is_some_and(|groups| groups.contains(&group_id))
        {
            return root;
        }
        let Some(group_rect) = snapshot
            .rects
            .groups
            .iter()
            .find(|candidate| candidate.group == group_id)
            .copied()
        else {
            return root;
        };
        let Some(strip) = snapshot
            .rects
            .tab_strips
            .iter()
            .find(|candidate| candidate.group == group_id)
            .copied()
        else {
            return root;
        };
        if !snapshot.group_presentations.contains_key(&group_id) {
            return root;
        }

        let transferred_panels = self.controller.update(cx, |controller, _| {
            controller.transferred_panel_views_from(window.window_handle().window_id())
        });
        let transfer = PanelTransferPresentation::new(
            &snapshot.group_presentations,
            &transferred_panels,
        );
        let mut rendered_presentations = snapshot.group_presentations.clone();
        transfer.apply_to_group_presentations(&mut rendered_presentations);
        let Some(presentations) = rendered_presentations.get(&group_id) else {
            return root;
        };

        let dragged_group = self.controller.update(cx, |controller, _| {
            controller.dragged_group_from(window.window_handle().window_id())
        });
        let local_group = GroupRect::new(
            group_id,
            Rect::new(Point::ZERO, group_rect.rect.size),
        );
        if dragged_group == Some(group_id)
            || transfer.groups_without_visible_tabs.contains(&group_id)
        {
            root = root.child(DockGroupView::render_placeholder(
                local_group,
                Point::ZERO,
                snapshot.metrics,
                self.theme,
            ));
            if snapshot.drag_preview.is_some() {
                root = root.child(render_floating_drop_overlay(
                    snapshot,
                    group_rect.rect.origin,
                    self.theme,
                ));
            }
            return root;
        }

        let local_strip = dock_core::TabStripRect::new(
            group_id,
            floating_part_local_rect(strip.rect, group_rect.rect.origin),
        );
        let visible_headers = transfer.visible_tab_headers(
            &snapshot.rects.tab_headers,
            &snapshot.rects.tab_strips,
            &snapshot.rects.tab_overflows,
            &rendered_presentations,
            &snapshot
                .column_groups
                .values()
                .flat_map(|groups| groups.iter().copied())
                .collect(),
            snapshot.metrics,
        );
        let local_headers = visible_headers
            .into_iter()
            .filter(|header| header.group == group_id)
            .map(|header| {
                dock_core::TabHeaderRect::new(
                    group_id,
                    header.panel,
                    header.index,
                    floating_part_local_rect(header.rect, group_rect.rect.origin),
                )
            })
            .collect::<Vec<_>>();
        let local_overflow = snapshot
            .rects
            .tab_overflows
            .iter()
            .find(|overflow| overflow.group == group_id)
            .cloned()
            .map(|mut overflow| {
                overflow.button_rect =
                    floating_part_local_rect(overflow.button_rect, group_rect.rect.origin);
                overflow
            });

        let pointer_input = render_floating_pointer_input(
            self.floating_group(),
            self.manager.clone(),
            self.controller.clone(),
            window.window_handle().window_id(),
            Some(snapshot.floating.rect),
            floating_dock_component_tab_zones(snapshot),
            None,
            true,
            self.tab_press.clone(),
            self.group_press.clone(),
            self.pointer_capture.clone(),
            Some(DockWindowPart::Group { group: group_id }),
            group_rect.rect.origin,
        );

        let toggle_host = cx.entity().downgrade();
        let toggle_context_menu: GroupMenuToggle = Rc::new(move |target, anchor, app| {
            if let Some(host) = toggle_host.upgrade() {
                host.update(app, |host, host_cx| {
                    host.open_context_menu = if host
                        .open_context_menu
                        .as_ref()
                        .is_some_and(|open| open.target == target)
                    {
                        None
                    } else {
                        Some(FloatingHostOpenMenu {
                            target,
                            anchor,
                            open_submenu: None,
                        })
                    };
                    host_cx.notify();
                });
            }
        });

        let group_ids = [group_id];
        let active_panels =
            active_panels_for_groups(group_ids, &rendered_presentations, dragged_group);
        let panel_rects = transfer
            .panel_rects_with_active_replacements(&snapshot.rects.panels)
            .into_iter()
            .filter(|panel| active_panels.contains(&panel.panel))
            .map(|panel| {
                let local = dock_core::PanelRect::new(
                    panel.panel,
                    floating_part_local_rect(panel.rect, group_rect.rect.origin),
                );
                let view = self.controller.update(cx, |controller, panel_cx| {
                    controller.panel_view(panel.panel, window, panel_cx)
                });
                (local, view)
            })
            .collect::<Vec<_>>();
        let rendered_panels = panel_rects
            .iter()
            .filter_map(|(rect, view)| view.as_ref().map(|_| rect.panel))
            .collect::<HashSet<_>>();
        let window_id = window.window_handle().window_id();
        // A new Group HWND may render synchronously while the manager is still
        // reconciling it. Defer both the handoff lookup and its confirmation.
        let manager = self.manager.clone();
        let floating_group = self.floating_group();
        window.on_next_frame(move |_, app| {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let Some(required_panels) = manager.update(app, |manager, manager_cx| {
                manager.pending_drag_host_handoff_panels(
                    floating_group,
                    window_id,
                    manager_cx,
                )
            }) else {
                return;
            };
            if !required_panels
                .iter()
                .all(|panel| rendered_panels.contains(panel))
            {
                return;
            }
            let _ = manager.update(app, |manager, manager_cx| {
                manager.notify_drag_host_handoff_rendered(
                    floating_group,
                    window_id,
                    &required_panels,
                    manager_cx,
                );
            });
        });

        root = root
            .child(pointer_input)
            .child(DockGroupView::render_core(
                Some(column),
                local_group,
                local_strip,
                &local_headers,
                local_overflow,
                presentations,
                Point::ZERO,
                snapshot.metrics,
                0.0,
                self.controller.clone(),
                self.theme,
                toggle_context_menu,
            ))
            .child(PanelHost::render(panel_rects, Point::ZERO, self.theme));

        if snapshot.drag_preview.is_some() {
            root = root.child(render_floating_drop_overlay(
                snapshot,
                group_rect.rect.origin,
                self.theme,
            ));
        }

        if let Some(open) = self.open_context_menu.as_ref()
            && context_menu_target_exists(
                open.target,
                &rendered_presentations,
                &snapshot.column_ids,
                Some(&snapshot.rects),
            )
        {
            let dismiss_host = cx.entity().downgrade();
            let dismiss_context_menu: ContextMenuDismiss = Rc::new(move |app| {
                if let Some(host) = dismiss_host.upgrade() {
                    host.update(app, |host, host_cx| {
                        host.open_context_menu = None;
                        host_cx.notify();
                    });
                }
            });
            let submenu_host = cx.entity().downgrade();
            let toggle_context_submenu: ContextMenuSubmenuToggle = Rc::new(move |index, app| {
                if let Some(host) = submenu_host.upgrade() {
                    host.update(app, |host, host_cx| {
                        if let Some(open) = host.open_context_menu.as_mut() {
                            open.open_submenu = index;
                            host_cx.notify();
                        }
                    });
                }
            });
            let entries = build_context_menu_entries(
                open.target,
                &snapshot.rects,
                &rendered_presentations,
                &snapshot.column_modes,
                self.controller.clone(),
                false,
            );
            if !entries.is_empty() {
                root = root.child(ContextMenu::render(
                    "floating-group-context-menu",
                    Rect::new(Point::ZERO, group_rect.rect.size),
                    open.anchor,
                    &entries,
                    open.open_submenu,
                    ContextMenuCallbacks {
                        on_submenu: toggle_context_submenu,
                        on_dismiss: dismiss_context_menu,
                    },
                    self.theme,
                ));
            }
        }

        let manager_for_escape = self.manager.clone();
        let controller_for_escape = self.controller.clone();
        let tab_press_for_escape = self.tab_press.clone();
        let group_press_for_escape = self.group_press.clone();
        let pointer_capture_for_escape = self.pointer_capture.clone();
        let context_menu_is_open = self.open_context_menu.is_some();
        root = root.capture_key_down(move |event, window, app| {
            handle_floating_escape(
                event,
                context_menu_is_open,
                &manager_for_escape,
                &controller_for_escape,
                &tab_press_for_escape,
                &group_press_for_escape,
                &pointer_capture_for_escape,
                window,
                app,
            );
        });

        root
    }
}

