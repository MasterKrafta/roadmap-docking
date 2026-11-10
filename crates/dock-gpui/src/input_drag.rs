use dock_core::{
    ColumnRect, ComputeRectsError, DragDropAction, DragDropRequest, DragHandle, DragPayload,
    DragState, DropSource, DropTarget, FloatingGroupId, Node, NodeId,
    LayoutRects, PanelId, Point, Rect, Size,
};
use gpui::{App, Entity, WindowId};

use crate::{
    dock_components::DockTab, DockController, DragHostManager, DragSourceHost,
    FloatingWindowManager,
};

/// Identifies a FloatingHost tab gesture across pointer updates and DragHost detach.
#[derive(Clone, Copy)]
pub(crate) struct FloatingTabDrag {
    pub(crate) source_window_id: WindowId,
    pub(crate) source_group: FloatingGroupId,
    pub(crate) panel: PanelId,
    pub(crate) press_origin: Point,
    pub(crate) layout_version: u64,
}

/// Begins a drag from an adapter-provided point in Core's host-local coordinates.
pub(crate) fn begin_drag_at(
    controller: &Entity<DockController>,
    payload: DragPayload,
    handle: DragHandle,
    position: Point,
    source_host: DragSourceHost,
    source_window_id: WindowId,
    app: &mut App,
) {
    let result = controller.update(app, |controller, cx| {
        // A layout change may have invalidated a gesture that did not receive
        // its normal release/cancel event. Clear that stale state before arming
        // the new pointer-down; a still-current active gesture is left intact
        // and DragSession will reject an overlapping start.
        if !controller.ensure_pointer_interaction_current() {
            cx.notify();
        }

        controller.begin_content_drag_interaction(
            payload,
            handle,
            position,
            source_host,
            source_window_id,
        )
    });

    match result {
        Ok(()) => {
            tracing::trace!(
                ?payload,
                ?handle,
                source_window_id = ?source_window_id,
                "dock drag gesture started"
            );
        }
        Err(error) => {
            tracing::warn!(
                %error,
                ?payload,
                ?handle,
                "DockCore rejected the dock drag gesture"
            );
        }
    }
}

/// Dock content gestures that have left their source surface defer drop-target
/// tracking to DragHost's native polling loop. Same-strip PanelTab reorders are
/// handled separately before this resolver-routing decision.
fn main_drag_uses_drop_targets(handle: Option<DragHandle>) -> bool {
    !matches!(
        handle,
        Some(
            DragHandle::PanelTab
                | DragHandle::FlyoutPanel
                | DragHandle::GroupGrip
                | DragHandle::ColumnHeader
                | DragHandle::DockAreaHeader
                | DragHandle::FloatingTitle
        )
    )
}

/// Updates a drag using a host-local point, including points translated out of a rail scroll.
pub(crate) fn update_drag_at(
    controller: &Entity<DockController>,
    position: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, cx| {
        let (payload, was_pressed) = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } => (payload, true),
            DragState::Dragging { payload, .. } => (payload, false),
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return false;
            }
        };
        let handle = controller.drag().session().handle();

        if let Err(error) = controller.drag_mut().session_mut().pointer_move(position) {
            tracing::debug!(%error, "dock drag pointer update failed");
            cancel_drag(controller);
            cx.notify();
            return false;
        }

        if !matches!(
            controller.drag().session().state(),
            DragState::Dragging { .. }
        ) {
            // Keep Pressed alive until Core's configured threshold is crossed.
            // No resolver or drag preview exists before this transition.
            return false;
        }

        if handle == Some(DragHandle::PanelTab)
            && let DragPayload::Panel { panel } = payload
        {
            let target = main_panel_tab_reorder_target(controller, panel, position);
            if let Err(error) = controller.drag_mut().session_mut().set_drop_target(target) {
                tracing::debug!(%error, "could not update same-group tab insertion target");
                cancel_drag(controller);
                cx.notify();
                return was_pressed;
            }
            cx.notify();
            return was_pressed;
        }

        // Detached content drags resolve targets in DragHost's tracking loop.
        // Floating-title gestures move their own window and never resolve dock targets.
        if !main_drag_uses_drop_targets(handle) {
            cx.notify();
            return was_pressed;
        }

        let Some(source) = drop_source(payload) else {
            tracing::debug!("dock drag source no longer exists");
            cancel_drag(controller);
            cx.notify();
            return was_pressed;
        };

        // Prepare exactly once: on the Pressed -> Dragging transition. All
        // later pointer moves reuse the resolver captured for this gesture.
        let target = match resolve_drop_target(controller, source, position, was_pressed) {
            Ok(target) => target,
            Err(error) => {
                tracing::debug!(%error, "could not resolve dock drag target");
                cancel_drag(controller);
                cx.notify();
                return was_pressed;
            }
        };
        if let Err(error) = controller.drag_mut().session_mut().set_drop_target(target) {
            tracing::debug!(%error, "could not update dock drag target");
            cancel_drag(controller);
            cx.notify();
            return was_pressed;
        }
        cx.notify();
        was_pressed
    })
}

/// Starts the temporary DragHost for an active PanelTab or FlyoutPanel gesture.
/// The source host and original pointer origin always come from DragHostSession;
/// the temporary window never becomes the semantic drag source.
pub(crate) fn begin_panel_drag_host(
    controller: &Entity<DockController>,
    drag_host_manager: &Entity<DragHostManager>,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (panel, origin, handle) = match controller.drag().session().state() {
            DragState::Dragging {
                payload: DragPayload::Panel { panel },
                origin,
                ..
            } => {
                let handle = controller.drag().session().handle()?;
                if !matches!(handle, DragHandle::PanelTab | DragHandle::FlyoutPanel) {
                    return None;
                }
                (panel, origin, handle)
            }
            _ => return None,
        };
        let host_session = controller.drag().host_session()?;
        let source_host = host_session.source_host();
        let source_window_id = host_session.source_window_id()?;
        let promote_on_float = matches!(
            (source_host, handle),
            (DragSourceHost::Main, DragHandle::PanelTab)
        );
        let preferred_size = controller.layout().panel_instance(panel)?.constraints.preferred_size;

        let source_rect = match source_host {
            DragSourceHost::Main => {
                let rects = controller.geometry().ok()?;
                match handle {
                    DragHandle::PanelTab => rects
                        .tab_headers
                        .iter()
                        .find(|header| header.panel == panel)
                        .map(|header| header.rect)?,
                    DragHandle::FlyoutPanel => rects
                        .flyout
                        .filter(|flyout| flyout.panel == panel)
                        .map(|flyout| flyout.rect)
                        .or_else(|| {
                            rects
                                .panels
                                .iter()
                                .find(|panel_rect| panel_rect.panel == panel)
                                .map(|panel_rect| panel_rect.rect)
                        })?,
                    _ => return None,
                }
            }
            DragSourceHost::Floating { group } => {
                let rects = controller.floating_geometry(source_window_id, group).ok()?;
                match handle {
                    DragHandle::PanelTab => rects
                        .tab_headers
                        .iter()
                        .find(|header| header.panel == panel)
                        .map(|header| header.rect)?,
                    DragHandle::FlyoutPanel => rects
                        .flyout
                        .filter(|flyout| flyout.panel == panel)
                        .map(|flyout| flyout.rect)
                        .or_else(|| {
                            rects
                                .panels
                                .iter()
                                .find(|panel_rect| panel_rect.panel == panel)
                                .map(|panel_rect| panel_rect.rect)
                        })?,
                    _ => return None,
                }
            }
        };
        let grab_offset = Point::new(
            (origin.x - source_rect.origin.x)
                .clamp(0.0, source_rect.size.width)
                .min(preferred_size.width),
            (origin.y - source_rect.origin.y)
                .clamp(0.0, source_rect.size.height)
                .min(preferred_size.height),
        );
        Some((panel, grab_offset, preferred_size, promote_on_float))
    });
    let Some((panel, grab_offset, preferred_size, promote_on_float)) = candidate else {
        tracing::debug!("Panel drag has no valid source geometry for DragHost");
        controller.update(app, |controller, cx| {
            if let Err(error) = controller.cancel_pointer_interaction() {
                tracing::debug!(%error, "could not cancel Panel drag without source geometry");
            }
            cx.notify();
        });
        return false;
    };

    match drag_host_manager.update(app, |manager, manager_cx| {
        if promote_on_float {
            manager.open_panel_tab_at_cursor(panel, grab_offset, preferred_size, manager_cx)
        } else {
            manager.open_panel_at_cursor(panel, grab_offset, preferred_size, manager_cx)
        }
    }) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "could not start Panel DragHost");
            controller.update(app, |controller, cx| {
                if let Err(cancel_error) = controller.cancel_pointer_interaction() {
                    tracing::debug!(%cancel_error, "could not cancel failed Panel DragHost start");
                }
                cx.notify();
            });
            false
        }
    }
}

/// Starts DragHost for an active GroupGrip gesture, transferring the Group's
/// active cached PanelView while preserving the complete tab-strip presentation.
pub(crate) fn begin_group_drag_host(
    controller: &Entity<DockController>,
    drag_host_manager: &Entity<DragHostManager>,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (group, origin) = match controller.drag().session().state() {
            DragState::Dragging {
                payload: DragPayload::Group { group },
                origin,
                ..
            } if controller.drag().session().handle() == Some(DragHandle::GroupGrip) => {
                (group, origin)
            }
            _ => return None,
        };
        let host_session = controller.drag().host_session()?;
        let source_host = host_session.source_host();
        let source_window_id = host_session.source_window_id()?;
        let group_rect = match source_host {
            DragSourceHost::Main => controller
                .geometry()
                .ok()?
                .groups
                .iter()
                .find(|rect| rect.group == group)
                .copied()?,
            DragSourceHost::Floating { group: floating } => controller
                .floating_geometry(source_window_id, floating)
                .ok()?
                .groups
                .iter()
                .find(|rect| rect.group == group)
                .copied()?,
        };
        let panel = match controller.layout().node(group) {
            Some(Node::Tabs(tabs)) => tabs.active_child().and_then(|child| {
                match controller.layout().node(child) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                }
            })?,
            _ => return None,
        };
        let grab_offset = Point::new(
            (origin.x - group_rect.rect.origin.x)
                .clamp(0.0, group_rect.rect.size.width),
            (origin.y - group_rect.rect.origin.y)
                .clamp(0.0, group_rect.rect.size.height),
        );
        Some((group, panel, grab_offset, group_rect.rect.size))
    });
    let Some((group, panel, grab_offset, logical_size)) = candidate else {
        tracing::debug!("GroupGrip drag has no valid source geometry for DragHost");
        controller.update(app, |controller, cx| {
            if let Err(error) = controller.cancel_pointer_interaction() {
                tracing::debug!(%error, "could not cancel Group drag without source geometry");
            }
            cx.notify();
        });
        return false;
    };

    match drag_host_manager.update(app, |manager, manager_cx| {
        manager.open_group_at_cursor(group, panel, grab_offset, logical_size, manager_cx)
    }) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "could not start Group DragHost");
            controller.update(app, |controller, cx| {
                if let Err(cancel_error) = controller.cancel_pointer_interaction() {
                    tracing::debug!(%cancel_error, "could not cancel failed Group DragHost start");
                }
                cx.notify();
            });
            false
        }
    }
}

/// Starts DragHost for a ColumnHeader gesture and transfers every active
/// Group PanelView while preserving the complete Column composition.
pub(crate) fn begin_column_drag_host(
    controller: &Entity<DockController>,
    drag_host_manager: &Entity<DragHostManager>,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (column, origin) = match controller.drag().session().state() {
            DragState::Dragging {
                payload: DragPayload::Column { column },
                origin,
                ..
            } if controller.drag().session().handle() == Some(DragHandle::ColumnHeader) => {
                (column, origin)
            }
            _ => return None,
        };
        let host_session = controller.drag().host_session()?;
        let source_host = host_session.source_host();
        let source_window_id = host_session.source_window_id()?;
        let column_rect = match source_host {
            DragSourceHost::Main => controller
                .geometry()
                .ok()?
                .columns
                .iter()
                .find(|rect| rect.column == column)
                .copied()?,
            DragSourceHost::Floating { group } => controller
                .floating_geometry(source_window_id, group)
                .ok()?
                .columns
                .iter()
                .find(|rect| rect.column == column)
                .copied()?,
        };
        let panels = controller.active_column_panel_ids(column)?;
        let grab_offset = Point::new(
            (origin.x - column_rect.rect.origin.x)
                .clamp(0.0, column_rect.rect.size.width),
            (origin.y - column_rect.rect.origin.y)
                .clamp(0.0, column_rect.rect.size.height),
        );
        Some((column, panels, grab_offset, column_rect.rect.size))
    });
    let Some((column, panels, grab_offset, logical_size)) = candidate else {
        tracing::debug!("ColumnHeader drag has no valid source geometry for DragHost");
        controller.update(app, |controller, cx| {
            if let Err(error) = controller.cancel_pointer_interaction() {
                tracing::debug!(%error, "could not cancel Column drag without source geometry");
            }
            cx.notify();
        });
        return false;
    };

    match drag_host_manager.update(app, |manager, manager_cx| {
        manager.open_column_at_cursor(column, panels, grab_offset, logical_size, manager_cx)
    }) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "could not start Column DragHost");
            controller.update(app, |controller, cx| {
                if let Err(cancel_error) = controller.cancel_pointer_interaction() {
                    tracing::debug!(%cancel_error, "could not cancel failed Column DragHost start");
                }
                cx.notify();
            });
            false
        }
    }
}

/// Starts DragHost for an entire Main DockArea composition and transfers
/// every active cached PanelView belonging to that DockArea.
pub(crate) fn begin_dock_area_drag_host(
    controller: &Entity<DockController>,
    drag_host_manager: &Entity<DragHostManager>,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (side, origin) = match controller.drag().session().state() {
            DragState::Dragging {
                payload: DragPayload::DockArea { side },
                origin,
                ..
            } if controller.drag().session().handle() == Some(DragHandle::DockAreaHeader) => {
                (side, origin)
            }
            _ => return None,
        };
        let host_session = controller.drag().host_session()?;
        if host_session.source_host() != DragSourceHost::Main
            || host_session.source_window_id().is_none()
        {
            return None;
        }

        let column_rects = controller.geometry().ok()?.columns.clone();
        let area = controller.layout().main_area(side)?;
        let mut column_ids = Vec::new();
        collect_dock_area_columns(controller.layout(), area.root, &mut column_ids);
        let columns = column_ids
            .iter()
            .filter_map(|column| {
                column_rects
                    .iter()
                    .find(|rect| rect.column == *column)
                    .copied()
            })
            .collect::<Vec<_>>();
        let area_rect = dock_area_bounds(&columns)?;
        let panels = controller.active_dock_area_panel_ids(side)?;

        let grab_offset = Point::new(
            (origin.x - area_rect.origin.x).clamp(0.0, area_rect.size.width),
            (origin.y - area_rect.origin.y).clamp(0.0, area_rect.size.height),
        );
        Some((side, panels, grab_offset, area_rect.size))
    });

    let Some((side, panels, grab_offset, logical_size)) = candidate else {
        tracing::debug!("DockAreaHeader drag has no valid source geometry for DragHost");
        controller.update(app, |controller, cx| {
            if let Err(error) = controller.cancel_pointer_interaction() {
                tracing::debug!(%error, "could not cancel DockArea drag without source geometry");
            }
            cx.notify();
        });
        return false;
    };

    match drag_host_manager.update(app, |manager, manager_cx| {
        manager.open_dock_area_at_cursor(side, panels, grab_offset, logical_size, manager_cx)
    }) {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "could not start DockArea DragHost");
            controller.update(app, |controller, cx| {
                if let Err(cancel_error) = controller.cancel_pointer_interaction() {
                    tracing::debug!(%cancel_error, "could not cancel failed DockArea DragHost start");
                }
                cx.notify();
            });
            false
        }
    }
}


/// Updates a tab drag that started inside a floating host. The resolver is
/// built from that host's own local geometry, not from the main DockHost.
pub(crate) fn update_floating_tab_drag(
    controller: &Entity<DockController>,
    drag: FloatingTabDrag,
    position: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, cx| {
        if controller.layout().version() != drag.layout_version {
            if matches!(
                controller.drag().session().state(),
                DragState::Pressed {
                    payload: DragPayload::Panel { panel: active },
                    ..
                } | DragState::Dragging {
                    payload: DragPayload::Panel { panel: active },
                    ..
                } if active == drag.panel
            ) {
                cancel_drag(controller);
            }
            return false;
        }

        let payload = DragPayload::Panel { panel: drag.panel };
        match controller.drag().session().state() {
            DragState::Idle => {
                if let Err(error) = controller.begin_content_drag_interaction(
                    payload,
                    DragHandle::PanelTab,
                    drag.press_origin,
                    DragSourceHost::Floating { group: drag.source_group },
                    drag.source_window_id,
                ) {
                    tracing::debug!(%error, "could not begin floating tab drag");
                    return false;
                }
            }
            DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                if let Err(error) = controller.drag_mut().session_mut().reset() {
                    tracing::debug!(%error, "could not reset previous floating tab drag");
                    return false;
                }
                controller.drag_mut().clear_resolver();
                controller.drag_mut().clear_layout_version();
                if let Err(error) = controller.begin_content_drag_interaction(
                    payload,
                    DragHandle::PanelTab,
                    drag.press_origin,
                    DragSourceHost::Floating { group: drag.source_group },
                    drag.source_window_id,
                ) {
                    tracing::debug!(%error, "could not begin floating tab drag");
                    return false;
                }
            }
            DragState::Pressed {
                payload: active,
                origin,
                ..
            }
            | DragState::Dragging {
                payload: active,
                origin,
                ..
            } if active == payload && origin == drag.press_origin => {}
            _ => return false,
        }

        if let Err(error) = controller.drag_mut().session_mut().pointer_move(position) {
            tracing::debug!(%error, "floating tab drag pointer update failed");
            cancel_drag(controller);
            cx.notify();
            return false;
        }

        if !matches!(controller.drag().session().state(), DragState::Dragging { .. }) {
            return false;
        }

        let source = DropSource::Panel { panel: drag.panel };
        let local_reorder_target = floating_panel_tab_reorder_target(
            controller,
            drag.source_window_id,
            drag.source_group,
            drag.panel,
            position,
        );
        let target = if local_reorder_target.is_some() {
            local_reorder_target
        } else {
            match controller
                .prepare_floating_window_drop_resolver(
                    drag.source_window_id,
                    drag.source_group,
                    source,
                )
                .and_then(|resolver| resolver.resolve(position))
            {
                Ok(target) => target,
                Err(error) => {
                    tracing::debug!(%error, "could not resolve floating tab drop target");
                    cancel_drag(controller);
                    cx.notify();
                    return false;
                }
            }
        };
        if let Err(error) = controller.drag_mut().session_mut().set_drop_target(target) {
            tracing::debug!(%error, "could not update floating tab drop target");
            cancel_drag(controller);
            cx.notify();
            return false;
        }

        cx.notify();
        true
    })
}

/// Computes a same-group insertion slot for a Main-host PanelTab gesture.
/// The tab remains in the source host while the pointer stays inside its tab drag zone.
fn main_panel_tab_reorder_target(
    controller: &mut DockController,
    panel: PanelId,
    position: Point,
) -> Option<DropTarget> {
    let (group, group_rect, headers) = {
        let rects = controller.geometry().ok()?;
        let source = rects.tab_headers.iter().find(|header| header.panel == panel)?;
        let group = source.group;
        let group_rect = rects
            .groups
            .iter()
            .find(|group_rect| group_rect.group == group)?
            .rect;
        let headers = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group)
            .copied()
            .collect::<Vec<_>>();
        (group, group_rect, headers)
    };

    let drag_zone = DockTab::drag_zone(group_rect, controller.metrics());
    if DockTab::leaves_strip_zone(drag_zone, position) {
        return None;
    }

    let tab_count = match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => return None,
    };
    Some(DropTarget::Tabs {
        group,
        index: tab_insertion_index(&headers, tab_count, position.x),
    })
}

/// Computes a same-group insertion slot for a FloatingHost tab gesture.
/// Multi-tab groups use their actual TabStrip; a single tab uses the same
/// reduced drag zone as the FloatingHost tear-off pipeline.
fn floating_panel_tab_reorder_target(
    controller: &mut DockController,
    window_id: WindowId,
    floating_group: FloatingGroupId,
    panel: PanelId,
    position: Point,
) -> Option<DropTarget> {
    let (group, strip_rect, group_rect, headers) = {
        let rects = controller
            .floating_geometry(window_id, floating_group)
            .ok()?;
        let source = rects
            .tab_headers
            .iter()
            .find(|header| header.panel == panel)?;
        let group = source.group;
        let strip_rect = rects
            .tab_strips
            .iter()
            .find(|strip| strip.group == group)?
            .rect;
        let group_rect = rects
            .groups
            .iter()
            .find(|group_rect| group_rect.group == group)?
            .rect;
        let headers = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group)
            .copied()
            .collect::<Vec<_>>();
        (group, strip_rect, group_rect, headers)
    };

    let tab_count = match controller.layout().node(group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => return None,
    };
    let allowed_zone = if tab_count == 1 {
        DockTab::drag_zone(group_rect, controller.metrics())
    } else {
        strip_rect
    };
    if DockTab::leaves_strip_zone(allowed_zone, position) {
        return None;
    }

    Some(DropTarget::Tabs {
        group,
        index: tab_insertion_index(&headers, tab_count, position.x),
    })
}

/// Converts the pointer's horizontal position into an insertion slot.
/// Header midpoints split the slots; the end of the visible strip maps to append.
fn tab_insertion_index(
    headers: &[dock_core::TabHeaderRect],
    tab_count: usize,
    pointer_x: f32,
) -> usize {
    headers
        .iter()
        .filter(|header| pointer_x < header.rect.origin.x + header.rect.size.width * 0.5)
        .min_by_key(|header| header.index)
        .map(|header| header.index)
        .unwrap_or(tab_count)
}

/// Finishes a tab drag that remains inside its source TabStrip. Leaving that
/// region promotes the gesture to DragHost through the FloatingHost input pipeline.
pub(crate) fn finish_floating_tab_drag(
    controller: &Entity<DockController>,
    window_id: WindowId,
    source_group: FloatingGroupId,
    panel: PanelId,
    position: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, cx| {
        let is_active_panel_drag = matches!(
            controller.drag().session().state(),
            DragState::Dragging {
                payload: DragPayload::Panel { panel: active },
                ..
            } if active == panel
        );
        if !is_active_panel_drag {
            return false;
        }

        if let Err(error) = controller.drag_mut().session_mut().pointer_move(position) {
            tracing::debug!(%error, "floating tab release position was rejected");
            cancel_drag(controller);
            cx.notify();
            return true;
        }

        let target = floating_panel_tab_reorder_target(
            controller,
            window_id,
            source_group,
            panel,
            position,
        )
        .or_else(|| {
            controller
                .prepare_floating_window_drop_resolver(
                    window_id,
                    source_group,
                    DropSource::Panel { panel },
                )
                .and_then(|resolver| resolver.resolve(position))
                .unwrap_or(None)
        });
        if let Err(error) = controller.drag_mut().session_mut().set_drop_target(target) {
            tracing::debug!(%error, "could not set final floating tab drop target");
            cancel_drag(controller);
            cx.notify();
            return true;
        }

        let request = match controller
            .drag_mut()
            .session_mut()
            .pointer_up(position, target)
        {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "floating tab drag release did not produce a request");
                None
            }
        };

        if let Some(request) = request {
            let action = {
                let metrics = controller.metrics();
                controller.layout().resolve_drag_drop(&request, &metrics)
            };
            match action {
                Ok(DragDropAction::NoOp) => {}
                Ok(DragDropAction::Command(command)) => {
                    if let Err(error) = controller.dispatch_in(command, cx) {
                        tracing::debug!(%error, "floating tab drop command was rejected");
                    }
                }
                Err(error) => controller.report_drop_resolution_error_in(error, cx),
            }
        }

        let _ = controller.drag_mut().session_mut().reset();
        controller.drag_mut().clear_resolver();
        controller.drag_mut().clear_layout_version();
        controller.drag_mut().finish_host_session();
        cx.notify();
        true
    })
}

/// Ends a drag from a host-local release point. It prepares a resolver only when
/// this release itself crosses the Core threshold; otherwise it reuses the
/// resolver captured at that transition. Core revalidates before one command.
pub(crate) fn finish_drag_at(
    controller: &Entity<DockController>,
    manager: &Entity<FloatingWindowManager>,
    position: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, cx| {
        let (payload, was_pressed) = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } => (payload, true),
            DragState::Dragging { payload, .. } => (payload, false),
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                let _ = controller.drag_mut().session_mut().reset();
                controller.drag_mut().clear_resolver();
                controller.drag_mut().clear_layout_version();
                controller.drag_mut().finish_host_session();
                return false;
            }
        };
        let handle = controller.drag().session().handle();

        if let Err(error) = controller.drag_mut().session_mut().pointer_move(position) {
            tracing::debug!(%error, "dock drag release position was rejected");
            cancel_drag(controller);
            cx.notify();
            return false;
        }

        // The release coordinate itself may cross the Core threshold even if
        // GPUI did not deliver an intermediate MouseMove event.
        let drag_started = matches!(
            controller.drag().session().state(),
            DragState::Dragging { .. }
        );
        let mut target = if !drag_started {
            None
        } else if handle == Some(DragHandle::PanelTab) {
            match payload {
                DragPayload::Panel { panel } => {
                    main_panel_tab_reorder_target(controller, panel, position)
                }
                _ => None,
            }
        } else if !main_drag_uses_drop_targets(handle) {
            None
        } else {
            match drop_source(payload) {
                Some(source) => {
                    match resolve_drop_target(controller, source, position, was_pressed) {
                        Ok(target) => target,
                        Err(error) => {
                            tracing::debug!(%error, "could not resolve dock drag release target");
                            None
                        }
                    }
                }
                None => {
                    tracing::debug!("dock drag source no longer exists at release");
                    None
                }
            }
        };

        // Keep DragPreview.target synchronized with the final pointer position
        // before asking DragSession to finish the gesture.
        if drag_started
            && (main_drag_uses_drop_targets(handle) || handle == Some(DragHandle::PanelTab))
            && let Err(error) = controller.drag_mut().session_mut().set_drop_target(target)
        {
            tracing::debug!(%error, "could not set final dock drop target");
            target = None;
        }

        let request = match controller
            .drag_mut()
            .session_mut()
            .pointer_up(position, target)
        {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "dock drag release did not produce a request");
                None
            }
        };

        if let Some(mut request) = request {
            if request.target == Some(DropTarget::FreeArea) {
                // These are desired model-space bounds, not a viewport-clamped
                // result. Core validates the source/area constraints and applies
                // floating-size and viewport fitting in its geometry pipeline.
                let desired_bounds = floating_bounds_for_request(controller, &request);
                request.floating_bounds = desired_bounds.and_then(|bounds| {
                    manager.update(cx, |manager, _| {
                        manager.screen_bounds_from_main_local(bounds)
                    })
                });
            }

            // Resolve every completed request in Core. NoOp is deliberately not
            // dispatched; Command carries the one and only mutation for this drop.
            let action = {
                let metrics = controller.metrics();
                controller.layout().resolve_drag_drop(&request, &metrics)
            };
            match action {
                Ok(DragDropAction::NoOp) => {}
                Ok(DragDropAction::Command(command)) => {
                    // dispatch_in delegates to exactly one dispatch and then
                    // publishes the resulting events to the GPUI entity.
                    if let Err(error) = controller.dispatch_in(command, cx) {
                        tracing::debug!(%error, "dock drop command was rejected");
                    }
                }
                Err(error) => {
                    controller.report_drop_resolution_error_in(error, cx);
                }
            }
        }

        let _ = controller.drag_mut().session_mut().reset();
        controller.drag_mut().clear_resolver();
        controller.drag_mut().clear_layout_version();
        controller.drag_mut().finish_host_session();
        cx.notify();
        drag_started
    })
}

fn resolve_drop_target(
    controller: &mut DockController,
    source: DropSource,
    point: Point,
    prepare_on_transition: bool,
) -> Result<Option<DropTarget>, ComputeRectsError> {
    if prepare_on_transition {
        // This calls Layout::prepare_drop_resolver(source, &rects, &metrics)
        // and stores the owned resolver in DragController for this gesture.
        controller.prepare_drop_resolver(source)?.resolve(point)
    } else {
        controller
            .drag()
            .resolver()
            .ok_or(ComputeRectsError::InvalidLayoutState)?
            .resolve(point)
    }
}

fn cancel_drag(controller: &mut DockController) {
    if let Err(error) = controller.cancel_pointer_interaction() {
        tracing::debug!(%error, "could not cancel invalid dock drag");
    }
}

pub(crate) fn drop_source(payload: DragPayload) -> Option<DropSource> {
    match payload {
        DragPayload::Panel { panel } => Some(DropSource::Panel { panel }),
        DragPayload::Group { group } => Some(DropSource::Group { group }),
        DragPayload::Column { column } => Some(DropSource::Column { column }),
        // DockArea and whole-floating-window captures are routed by
        // FloatingWindowManager, not by the Main-host resolver.
        DragPayload::DockArea { .. } | DragPayload::FloatingGroup { .. } => None
    }
}

/// Calculates desired bounds in the persistent source host's local coordinates.
/// Unlike the main-window preview helper, this also reads a Floating source's own
/// geometry snapshot, so a later FreeArea drop preserves the original grab offset.
pub(crate) fn floating_bounds_for_drag(
    controller: &mut DockController,
    payload: DragPayload,
    pointer: Point,
    source_host: DragSourceHost,
    source_window_id: Option<WindowId>,
) -> Option<Rect> {
    let (active_payload, origin) = match controller.drag().session().state() {
        DragState::Pressed { payload, origin, .. }
        | DragState::Dragging { payload, origin, .. } => (payload, origin),
        _ => return None,
    };
    if active_payload != payload {
        return None;
    }

    let request = DragDropRequest {
        payload,
        handle: controller.drag().session().handle()?,
        origin,
        point: pointer,
        target: Some(DropTarget::FreeArea),
        floating_bounds: None,
    };
    floating_bounds_for_request_in_host(controller, &request, source_host, source_window_id)
}

/// Calculates the FreeArea preview using the active session's original press point.
/// Keeps the DockHost presentation API independent from DragDropRequest construction.
pub(crate) fn floating_bounds(
    controller: &mut DockController,
    payload: DragPayload,
    pointer: Point,
) -> Option<Rect> {
    let (active_payload, origin) = match controller.drag().session().state() {
        DragState::Pressed {
            payload, origin, ..
        }
        | DragState::Dragging {
            payload, origin, ..
        } => (payload, origin),
        _ => return None,
    };
    if active_payload != payload {
        return None;
    }

    let request = DragDropRequest {
        payload,
        handle: controller.drag().session().handle()?,
        origin,
        point: pointer,
        target: Some(DropTarget::FreeArea),
        floating_bounds: None,
    };
    floating_bounds_for_request(controller, &request)
}

fn floating_bounds_for_request(
    controller: &mut DockController,
    request: &DragDropRequest,
) -> Option<Rect> {
    floating_bounds_for_request_in_host(controller, request, DragSourceHost::Main, None)
}

fn floating_bounds_for_request_in_host(
    controller: &mut DockController,
    request: &DragDropRequest,
    source_host: DragSourceHost,
    source_window_id: Option<WindowId>,
) -> Option<Rect> {
    let column_header_height = controller.metrics().column_header_height;
    let dock_area_column_ids = match request.payload {
        DragPayload::DockArea { side } => {
            let root = controller.layout().main_area(side)?.root;
            let mut columns = Vec::new();
            collect_dock_area_columns(controller.layout(), root, &mut columns);
            Some(columns)
        }
        _ => None,
    };

    // Geometry is borrowed only long enough to extract the dragged item's
    // source rectangle. The full LayoutRects snapshot is never cloned.
    let source_rect = match source_host {
        DragSourceHost::Main => {
            let rects = controller.geometry().ok()?;
            source_rect_for_request(
                request,
                rects,
                dock_area_column_ids.as_deref(),
                column_header_height,
            )?
        }
        DragSourceHost::Floating { group } => {
            let window_id = source_window_id?;
            let rects = controller.floating_geometry(window_id, group).ok()?;
            source_rect_for_request(
                request,
                rects,
                dock_area_column_ids.as_deref(),
                column_header_height,
            )?
        }
    };

    let preferred_size = match request.payload {
        DragPayload::Panel { panel } => {
            controller
                .layout()
                .panel_instance(panel)?
                .constraints
                .preferred_size
        }
        DragPayload::Group { .. }
        | DragPayload::Column { .. }
        | DragPayload::DockArea { .. } => source_rect.size,
        DragPayload::FloatingGroup { group } => {
            controller.layout().floating_group(group)?.bounds.size
        }
    };

    desired_floating_bounds(source_rect, preferred_size, request.origin, request.point)
}

fn source_rect_for_request(
    request: &DragDropRequest,
    rects: &LayoutRects,
    dock_area_column_ids: Option<&[NodeId]>,
    column_header_height: f32,
) -> Option<Rect> {
    match (request.handle, request.payload) {
        (DragHandle::PanelTab, DragPayload::Panel { panel }) => rects
            .tab_headers
            .iter()
            .find(|header| header.panel == panel)
            .map(|header| header.rect),
        (DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => rects
            .flyout
            .filter(|flyout| flyout.panel == panel)
            .map(|flyout| flyout.rect)
            .or_else(|| {
                rects
                    .panels
                    .iter()
                    .find(|panel_rect| panel_rect.panel == panel)
                    .map(|panel_rect| panel_rect.rect)
            }),
        (DragHandle::GroupGrip, DragPayload::Group { group }) => rects
            .groups
            .iter()
            .find(|group_rect| group_rect.group == group)
            .map(|group_rect| group_rect.rect),
        (DragHandle::ColumnHeader, DragPayload::Column { column }) => {
            let source = rects
                .columns
                .iter()
                .find(|column_rect| column_rect.column == column)?
                .rect;
            column_tab_floating_bounds(source, request.origin, column_header_height)
        }
        (DragHandle::DockAreaHeader, DragPayload::DockArea { .. }) => {
            let ids = dock_area_column_ids?;
            let columns = ids
                .iter()
                .filter_map(|id| {
                    rects.columns.iter().find(|column| column.column == *id).copied()
                })
                .collect::<Vec<_>>();
            dock_area_bounds(&columns)
        }
        (
            DragHandle::FloatingCapture | DragHandle::FloatingTitle,
            DragPayload::FloatingGroup { group },
        ) => rects
            .floating_groups
            .iter()
            .find(|floating| floating.group == group)
            .map(|floating| floating.rect),
        _ => None,
    }
}

fn collect_dock_area_columns(
    layout: &dock_core::Layout,
    node: NodeId,
    columns: &mut Vec<NodeId>,
) {
    match layout.node(node) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_dock_area_columns(layout, *child, columns);
            }
        }
        Some(Node::Column(_)) => columns.push(node),
        _ => {}
    }
}

fn dock_area_bounds(columns: &[ColumnRect]) -> Option<Rect> {
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

/// Preserves the full Column rectangle for a ColumnHeader drag. The fallback
/// crop keeps older content-row drag origins from carrying the shared header band.
fn column_tab_floating_bounds(
    source: Rect,
    origin: Point,
    header_height: f32,
) -> Option<Rect> {
    let hidden_height = header_height.max(0.0).min(source.size.height);
    if origin.y < source.origin.y + hidden_height {
        return Some(source);
    }

    let height = source.size.height - hidden_height;
    if height <= 0.0 {
        return None;
    }

    Some(Rect::new(
        Point::new(source.origin.x, source.origin.y + hidden_height),
        Size::new(source.size.width, height),
    ))
}

/// Builds an adapter-side bounds proposal without applying viewport or
/// panel-constraint normalization. Core owns those decisions.
fn desired_floating_bounds(
    source_rect: Rect,
    preferred_size: Size,
    press_origin: Point,
    pointer: Point,
) -> Option<Rect> {
    source_rect.validate_non_empty().ok()?;
    preferred_size.validate_positive().ok()?;
    press_origin.validate().ok()?;
    pointer.validate().ok()?;

    let grab_offset_x = (press_origin.x - source_rect.origin.x)
        .clamp(0.0, source_rect.size.width)
        .min(preferred_size.width);
    let grab_offset_y = (press_origin.y - source_rect.origin.y)
        .clamp(0.0, source_rect.size.height)
        .min(preferred_size.height);

    let bounds = Rect::new(
        Point::new(pointer.x - grab_offset_x, pointer.y - grab_offset_y),
        preferred_size,
    );
    bounds.validate_non_empty().ok()?;
    Some(bounds)
}

#[cfg(test)]
mod main_drop_resolution_tests {
    use dock_core::DragHandle;

    use super::main_drag_uses_drop_targets;

    #[test]
    fn detached_content_drags_defer_drop_target_resolution_to_drag_host() {
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::PanelTab)));
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::FlyoutPanel)));
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::GroupGrip)));
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::ColumnHeader)));
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::DockAreaHeader)));
        assert!(!main_drag_uses_drop_targets(Some(DragHandle::FloatingTitle)));
    }

    #[test]
    fn unrelated_main_drag_handles_keep_drop_target_resolution() {
        assert!(main_drag_uses_drop_targets(None));
    }
}

#[cfg(test)]
mod floating_bounds_tests {
    use dock_core::{Point, Rect, Size};

    use super::{column_tab_floating_bounds, desired_floating_bounds};

    #[test]
    fn desired_bounds_preserve_the_cursor_offset_without_viewport_clamping() {
        let source = Rect::new(Point::new(100.0, 200.0), Size::new(300.0, 180.0));
        let bounds = desired_floating_bounds(
            source,
            Size::new(640.0, 480.0),
            Point::new(140.0, 225.0),
            Point::new(1_400.0, 950.0),
        )
        .unwrap();

        assert_eq!(
            bounds,
            Rect::new(Point::new(1_360.0, 925.0), Size::new(640.0, 480.0))
        );
    }

    #[test]
    fn dock_tab_detach_removes_column_header_area_and_preserves_grab_offset() {
        let source = Rect::new(Point::new(100.0, 200.0), Size::new(300.0, 500.0));
        let pointer_origin = Point::new(120.0, 250.0);
        let detached = column_tab_floating_bounds(source, pointer_origin, 32.0).unwrap();

        assert_eq!(
            detached,
            Rect::new(Point::new(100.0, 232.0), Size::new(300.0, 468.0))
        );

        let floating = desired_floating_bounds(
            detached,
            detached.size,
            pointer_origin,
            Point::new(500.0, 600.0),
        )
        .unwrap();
        assert_eq!(
            floating,
            Rect::new(Point::new(480.0, 582.0), Size::new(300.0, 468.0))
        );
    }

    #[test]
    fn column_header_drag_keeps_the_full_column_bounds() {
        let source = Rect::new(Point::new(100.0, 200.0), Size::new(300.0, 500.0));
        assert_eq!(
            column_tab_floating_bounds(source, Point::new(120.0, 220.0), 32.0),
            Some(source),
        );
    }

    #[test]
    fn desired_bounds_keep_the_grab_point_inside_a_smaller_floating_item() {
        let source = Rect::new(Point::new(10.0, 20.0), Size::new(400.0, 300.0));
        let bounds = desired_floating_bounds(
            source,
            Size::new(160.0, 100.0),
            Point::new(390.0, 290.0),
            Point::new(700.0, 600.0),
        )
        .unwrap();

        assert_eq!(
            bounds,
            Rect::new(Point::new(540.0, 500.0), Size::new(160.0, 100.0))
        );
    }

}

#[cfg(test)]
mod tab_insertion_tests {
    use dock_core::{NodeId, PanelId, Point, Rect, Size};
    use slotmap::Key;

    use super::tab_insertion_index;

    #[test]
    fn tab_insertion_index_uses_header_midpoints_and_appends_at_the_end() {
        let group = NodeId::null();
        let headers = [
            dock_core::TabHeaderRect::new(
                group,
                PanelId::null(),
                0,
                Rect::new(Point::new(10.0, 10.0), Size::new(80.0, 24.0)),
            ),
            dock_core::TabHeaderRect::new(
                group,
                PanelId::null(),
                1,
                Rect::new(Point::new(90.0, 10.0), Size::new(80.0, 24.0)),
            ),
        ];

        assert_eq!(tab_insertion_index(&headers, 2, 40.0), 0);
        assert_eq!(tab_insertion_index(&headers, 2, 60.0), 1);
        assert_eq!(tab_insertion_index(&headers, 2, 150.0), 2);
    }
}
