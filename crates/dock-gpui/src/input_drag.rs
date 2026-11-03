use dock_core::{
    CommandValue, ComputeRectsError, DockCommand, DragDropAction, DragDropRequest, DragHandle,
    DragPayload, DragState, DropSource, DropTarget, FloatingGroupId, Node, PanelId, Point, Rect,
    Size,
};
use gpui::{App, Entity, WindowId};

use crate::{DockController, FloatingWindowManager};

/// Identifies the floating tab gesture across pointer updates and tear-off.
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
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, _| {
        controller.begin_drag_interaction(payload, handle, position)
    }) {
        tracing::debug!(%error, "could not begin dock drag gesture");
    }
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

        // A floating title moves its own window. It never enters dock target
        // resolution and therefore never owns a DropZoneResolver.
        if handle == Some(DragHandle::FloatingTitle) {
            cx.notify();
            return was_pressed;
        }

        let Some(source) = drop_source(controller, payload) else {
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
                if let Err(error) = controller.begin_drag_interaction(
                    payload,
                    DragHandle::PanelTab,
                    drag.press_origin,
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
                if let Err(error) = controller.begin_drag_interaction(
                    payload,
                    DragHandle::PanelTab,
                    drag.press_origin,
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
        let target = match controller
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

/// Detaches one tab after its pointer leaves the source floating host. The
/// existing tear-off transaction then continues as a native move of the new
/// single-panel floating group.
pub(crate) fn begin_floating_tab_tearoff(
    controller: &Entity<DockController>,
    manager: &Entity<FloatingWindowManager>,
    drag: FloatingTabDrag,
    pointer: Point,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (active_panel, origin) = match controller.drag().session().state() {
            DragState::Dragging {
                payload: DragPayload::Panel { panel },
                origin,
                ..
            } => (panel, origin),
            _ => return None,
        };
        if active_panel != drag.panel || origin != drag.press_origin {
            return None;
        }

        let (panel_node, preferred_size) = {
            let layout = controller.layout();
            let panel_node = layout.nodes().find_map(|(id, node)| {
                matches!(node, Node::Panel(value) if value.panel_id == drag.panel).then_some(id)
            })?;
            let preferred_size = layout.panel_instance(drag.panel)?.constraints.preferred_size;
            (panel_node, preferred_size)
        };
        let header_rect = controller
            .floating_geometry(drag.source_window_id, drag.source_group)
            .ok()?
            .tab_headers
            .iter()
            .find(|header| header.panel == drag.panel)?
            .rect;
        Some((panel_node, header_rect, preferred_size, origin))
    });
    let Some((panel_node, header_rect, preferred_size, origin)) = candidate else {
        return false;
    };

    let Some(local_bounds) =
        desired_floating_bounds(header_rect, preferred_size, origin, pointer)
    else {
        return false;
    };
    let model_bounds = manager
        .update(app, |manager, _| {
            manager.screen_bounds_from_floating_local(drag.source_group, local_bounds)
        });
    let Some(model_bounds) = model_bounds else {
        tracing::warn!("could not convert floating tab bounds into screen coordinates");
        return false;
    };

    let detached = controller.update(app, |controller, cx| {
        if !controller.begin_tearoff_transaction() {
            return None;
        }
        let outcome = match controller.dispatch_tearoff_float_in(
            DockCommand::Float {
                node: panel_node,
                bounds: model_bounds,
            },
            cx,
        ) {
            Ok(outcome) => outcome,
            Err(_) => return None,
        };
        match outcome.actual_value {
            Some(CommandValue::FloatingGroup(group))
                if controller.continue_tearoff_as_floating_capture(group).is_ok() =>
            {
                Some(group)
            }
            _ => {
                let _ = controller.cancel_pointer_interaction_on_escape();
                controller.flush_events(cx);
                cx.notify();
                None
            }
        }
    });
    let Some(group) = detached else {
        return false;
    };

    let started = manager.update(app, |manager, manager_cx| {
        manager.begin_floating_tab_tearoff_move(group, model_bounds, manager_cx)
    });
    if !started {
        let _ = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(cx);
            cx.notify();
            result
        });
        return false;
    }
    true
}

/// Finishes a tab drag that remains inside its source floating host. Drops that
/// leave the host are promoted to a native tear-off by begin_floating_tab_tearoff.
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

        let target = controller
            .prepare_floating_window_drop_resolver(
                window_id,
                source_group,
                DropSource::Panel { panel },
            )
            .and_then(|resolver| resolver.resolve(position))
            .unwrap_or(None);
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
        cx.notify();
        true
    })
}

/// Detaches a drag-only composition on pointer-down and assigns its already-created
/// hidden PanelWindow as a temporary native drag surface. The native move itself
/// remains thresholded; releasing without moving cancels the open history transaction.
pub(crate) fn begin_native_tearoff_on_press(
    controller: &Entity<DockController>,
    manager: &Entity<FloatingWindowManager>,
    pointer: Point,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (payload, handle) = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } => {
                (payload, controller.drag().session().handle()?)
            }
            _ => return None,
        };
        let local_bounds = floating_bounds(controller, payload, pointer)?;
        let command = match (handle, payload) {
            (DragHandle::ColumnHeader, DragPayload::Column { column }) => {
                DockCommand::Float {
                    node: column,
                    bounds: local_bounds,
                }
            }
            (DragHandle::GroupGrip, DragPayload::Group { group }) => DockCommand::Float {
                node: group,
                bounds: local_bounds,
            },
            (DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => {
                DockCommand::FloatPanelFromFlyout {
                    panel,
                    bounds: local_bounds,
                }
            }
            _ => return None,
        };
        Some((command, local_bounds))
    });
    let Some((command, local_bounds)) = candidate else {
        return false;
    };

    let model_bounds = manager.update(app, |manager, _| {
        manager.screen_bounds_from_main_local(local_bounds)
    });
    let Some(model_bounds) = model_bounds else {
        tracing::warn!("could not convert pressed tab bounds into physical screen coordinates");
        return false;
    };
    let command = match command {
        DockCommand::Float { node, .. } => DockCommand::Float {
            node,
            bounds: model_bounds,
        },
        DockCommand::FloatPanelFromFlyout { panel, .. } => {
            DockCommand::FloatPanelFromFlyout {
                panel,
                bounds: model_bounds,
            }
        }
        _ => return false,
    };

    let detached = controller.update(app, |controller, cx| {
        if !controller.begin_tearoff_transaction() {
            return None;
        }
        let outcome = match controller.dispatch_tearoff_float_in(command, cx) {
            Ok(outcome) => outcome,
            Err(_) => return None,
        };
        match outcome.actual_value {
            Some(CommandValue::FloatingGroup(group))
                if controller.continue_tearoff_as_floating_capture(group).is_ok() =>
            {
                Some(group)
            }
            _ => {
                let _ = controller.cancel_pointer_interaction_on_escape();
                controller.flush_events(cx);
                cx.notify();
                None
            }
        }
    });
    let Some(group) = detached else {
        return false;
    };

    let started = manager.update(app, |manager, manager_cx| {
        manager.begin_pending_tearoff_move(group, pointer, local_bounds, model_bounds, manager_cx)
    });
    if !started {
        let _ = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(cx);
            cx.notify();
            result
        });
        return false;
    }
    true
}

/// Positions the PanelId's existing hidden native window on pointer-down.
/// The source stays in its current Layout so tab reordering and docking remain available.
pub(crate) fn prepare_panel_tab_native_window_on_press(
    controller: &Entity<DockController>,
    manager: &Entity<FloatingWindowManager>,
    panel: PanelId,
    pointer: Point,
    app: &mut App,
) -> bool {
    let local_bounds = controller.update(app, |controller, _| {
        floating_bounds(
            controller,
            DragPayload::Panel { panel },
            pointer,
        )
    });
    let Some(local_bounds) = local_bounds else {
        return false;
    };

    manager.update(app, |manager, manager_cx| {
        manager.prepare_panel_tab_native_window_on_press(
            panel,
            pointer,
            local_bounds,
            manager_cx,
        )
    })
}

/// Turns an active main-window drag into a native floating window as soon as
/// the pointer leaves the DockHost viewport after crossing Core's drag threshold.
pub(crate) fn begin_native_tearoff(
    controller: &Entity<DockController>,
    manager: &Entity<FloatingWindowManager>,
    pointer: Point,
    app: &mut App,
) -> bool {
    let candidate = controller.update(app, |controller, _| {
        let (payload, handle) = match controller.drag().session().state() {
            DragState::Dragging { payload, .. } => (payload, controller.drag().session().handle()?),
            _ => return None,
        };
        let local_bounds = floating_bounds(controller, payload, pointer)?;
        let command = match (handle, payload) {
            (DragHandle::PanelTab, DragPayload::Panel { panel }) => {
                let node = controller.layout().nodes().find_map(|(id, node)| {
                    matches!(node, Node::Panel(value) if value.panel_id == panel).then_some(id)
                })?;
                DockCommand::Float { node, bounds: local_bounds }
            }
            (DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => {
                DockCommand::FloatPanelFromFlyout { panel, bounds: local_bounds }
            }
            (DragHandle::GroupGrip, DragPayload::Group { group }) => {
                DockCommand::Float { node: group, bounds: local_bounds }
            }
            (DragHandle::ColumnHeader, DragPayload::Column { column }) => {
                DockCommand::Float { node: column, bounds: local_bounds }
            }
            _ => return None,
        };
        Some((command, local_bounds))
    });

    let Some((command, local_bounds)) = candidate else {
        return false;
    };
    let model_bounds = manager.update(app, |manager, _| {
        manager.screen_bounds_from_main_local(local_bounds)
    });
    let Some(model_bounds) = model_bounds else {
        tracing::warn!("could not convert main-host bounds into physical screen coordinates");
        return false;
    };
    let command = match command {
        DockCommand::Float { node, .. } => DockCommand::Float { node, bounds: model_bounds },
        DockCommand::FloatPanelFromFlyout { panel, .. } => {
            DockCommand::FloatPanelFromFlyout { panel, bounds: model_bounds }
        }
        _ => return false,
    };

    let detached = controller.update(app, |controller, cx| {
        if !controller.begin_tearoff_transaction() {
            return None;
        }
        let outcome = match controller.dispatch_tearoff_float_in(command, cx) {
            Ok(outcome) => outcome,
            Err(_) => return None,
        };
        match outcome.actual_value {
            Some(CommandValue::FloatingGroup(group))
                if controller.continue_tearoff_as_floating_capture(group).is_ok() =>
            {
                Some(group)
            }
            _ => {
                let _ = controller.cancel_pointer_interaction_on_escape();
                controller.flush_events(cx);
                cx.notify();
                None
            }
        }
    });

    let Some(group) = detached else {
        return false;
    };
    let started = manager.update(app, |manager, manager_cx| {
        manager.begin_tearoff_move(group, pointer, local_bounds, model_bounds, manager_cx)
    });
    if !started {
        let _ = controller.update(app, |controller, cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(cx);
            cx.notify();
            result
        });
        return false;
    }
    true
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
        let mut target = if !drag_started || handle == Some(DragHandle::FloatingTitle) {
            None
        } else {
            match drop_source(controller, payload) {
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
            && handle != Some(DragHandle::FloatingTitle)
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

fn drop_source(controller: &DockController, payload: DragPayload) -> Option<DropSource> {
    match payload {
        DragPayload::Panel { panel } => Some(DropSource::Panel { panel }),
        DragPayload::Group { group } => Some(DropSource::Group { group }),
        DragPayload::Column { column } => Some(DropSource::Column { column }),
        DragPayload::FloatingGroup { group } => {
            let root = controller
                .layout()
                .floating_group(group)
                .and_then(|floating| controller.layout().dock_area(floating.area))
                .map(|area| area.root)?;
            Some(if matches!(controller.layout().node(root), Some(Node::Column(_))) {
                DropSource::Column { column: root }
            } else {
                DropSource::Group { group: root }
            })
        }
    }
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
    // Capture the source rectangle from the same Core geometry used by the
    // drag hit-testing pipeline. The press point then gives us the cursor's
    // offset within the dragged element.
    let source_rect = {
        let rects = controller.geometry().ok()?;
        match (request.handle, request.payload) {
            (DragHandle::PanelTab, DragPayload::Panel { panel }) => rects
                .tab_headers
                .iter()
                .find(|header| header.panel == panel)
                .map(|header| header.rect)?,
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
                })?,
            (DragHandle::GroupGrip, DragPayload::Group { group }) => rects
                .groups
                .iter()
                .find(|group_rect| group_rect.group == group)
                .map(|group_rect| group_rect.rect)?,
            (DragHandle::ColumnHeader, DragPayload::Column { column }) => rects
                .columns
                .iter()
                .find(|column_rect| column_rect.column == column)
                .map(|column_rect| column_rect.rect)?,
            (
                DragHandle::FloatingCapture | DragHandle::FloatingTitle,
                DragPayload::FloatingGroup { group },
            ) => rects
                .floating_groups
                .iter()
                .find(|floating| floating.group == group)
                .map(|floating| floating.rect)?,
            _ => return None,
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
        DragPayload::Group { .. } | DragPayload::Column { .. } => source_rect.size,
        DragPayload::FloatingGroup { group } => {
            controller.layout().floating_group(group)?.bounds.size
        }
    };

    desired_floating_bounds(source_rect, preferred_size, request.origin, request.point)
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
mod floating_bounds_tests {
    use dock_core::{Point, Rect, Size};

    use super::desired_floating_bounds;

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
