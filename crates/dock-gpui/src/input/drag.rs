use dock_core::{
    DockCommand, DragDropAction, DragHandle, DragPayload, DragState, DropSource,
    DropTarget, Point, Rect, Size,
};
use gpui::{App, Context, Entity, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

use crate::DockController;

/// Begins a Core drag session from a semantic panel or group source.
pub(crate) fn begin_drag(
    controller: &Entity<DockController>,
    payload: DragPayload,
    handle: DragHandle,
    event: &MouseDownEvent,
    app: &mut App,
) {
    begin_drag_at(
        controller,
        payload,
        handle,
        event_point(event.position.x, event.position.y),
        app,
    );
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
        controller
            .drag_mut()
            .session_mut()
            .pointer_down_with_handle(payload, handle, position)
    }) {
        tracing::debug!(%error, "could not begin dock drag gesture");
    }
}

/// Updates the shared drag state machine and resolves a fresh target while moving.
pub(crate) fn update_drag(
    controller: &Entity<DockController>,
    event: &MouseMoveEvent,
    app: &mut App,
) {
    if !event.dragging() {
        return;
    }
    update_drag_at(
        controller,
        event_point(event.position.x, event.position.y),
        app,
    );
}

/// Updates a drag using a host-local point, including points translated out of a rail scroll.
pub(crate) fn update_drag_at(
    controller: &Entity<DockController>,
    position: Point,
    app: &mut App,
) {
    let _ = controller.update(app, |controller, cx| {
        let payload = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } | DragState::Dragging { payload, .. } => payload,
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => return,
        };

        if let Err(error) = controller
            .drag_mut()
            .session_mut()
            .pointer_move(position)
        {
            tracing::debug!(%error, "dock drag pointer update failed");
            return;
        }

        if !matches!(controller.drag().session().state(), DragState::Dragging { .. }) {
            return;
        }

        let Some(source) = drop_source(controller, payload) else {
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            cx.notify();
            return;
        };
        let target = match controller.prepare_drop_resolver(source) {
            Ok(resolver) => match resolver.resolve(position) {
                Ok(target) => target,
                Err(error) => {
                    tracing::debug!(%error, "could not resolve dock drag target");
                    None
                }
            },
            Err(error) => {
                tracing::debug!(%error, "could not prepare dock drag resolver");
                None
            }
        };
        if let Err(error) = controller
            .drag_mut()
            .session_mut()
            .set_drop_target(target)
        {
            tracing::debug!(%error, "could not update dock drag target");
        }
        cx.notify();
    });
}

/// Ends a gesture and applies the single Core-resolved command, if the drop is valid.
pub(crate) fn finish_drag(
    controller: &Entity<DockController>,
    event: &MouseUpEvent,
    app: &mut App,
) {
    finish_drag_at(
        controller,
        event_point(event.position.x, event.position.y),
        app,
    );
}

/// Ends a drag from a host-local release point. A resolver is rebuilt/reused for
/// that exact point, and Core revalidates the resulting request before one command
/// is dispatched. A cancelled or invalid drop leaves the Layout untouched.
pub(crate) fn finish_drag_at(
    controller: &Entity<DockController>,
    position: Point,
    app: &mut App,
) {
    let _ = controller.update(app, |controller, cx| {
        let payload = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } | DragState::Dragging { payload, .. } => payload,
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                controller.drag_mut().clear_resolver();
                return;
            }
        };

        let _ = controller
            .drag_mut()
            .session_mut()
            .pointer_move(position);

        let target = if matches!(controller.drag().session().state(), DragState::Dragging { .. }) {
            drop_source(controller, payload).and_then(|source| {
                controller
                    .prepare_drop_resolver(source)
                    .ok()
                    .and_then(|resolver| resolver.resolve(position).ok().flatten())
            })
        } else {
            None
        };

        let request = controller
            .drag_mut()
            .session_mut()
            .pointer_up(position, target)
            .ok()
            .flatten();

        if let Some(mut request) = request {
            if request.target == Some(DropTarget::FreeArea) {
                request.floating_bounds = floating_bounds(controller, payload, position);
            }

            if request.target != Some(DropTarget::FreeArea) || request.floating_bounds.is_some() {
                let action = {
                    let metrics = controller.metrics();
                    controller.layout().resolve_drag_drop(&request, &metrics)
                };
                match action {
                    Ok(DragDropAction::NoOp) => {}
                    Ok(DragDropAction::Command(command)) => {
                        if let Err(error) = controller.dispatch_in(command, cx) {
                            tracing::debug!(%error, "dock drop command was rejected");
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "Core rejected the completed dock drag");
                    }
                }
            }
        }

        let _ = controller.drag_mut().session_mut().reset();
        controller.drag_mut().clear_resolver();
        cx.notify();
    });
}

fn drop_source(controller: &DockController, payload: DragPayload) -> Option<DropSource> {
    match payload {
        DragPayload::Panel { panel } => Some(DropSource::Panel { panel }),
        DragPayload::Group { group } => Some(DropSource::Group { group }),
        DragPayload::FloatingGroup { group } => {
            let root = controller
                .layout()
                .floating_group(group)
                .and_then(|floating| controller.layout().dock_area(floating.area))
                .map(|area| area.root)?;
            Some(DropSource::Group { group: root })
        }
    }
}

fn floating_bounds(
    controller: &mut DockController,
    payload: DragPayload,
    pointer: Point,
) -> Option<Rect> {
    let preferred = match payload {
        DragPayload::Panel { panel } => controller
            .layout()
            .panel_instance(panel)?
            .constraints
            .preferred_size,
        DragPayload::Group { group } => controller
            .geometry()
            .ok()?
            .groups
            .iter()
            .find(|rect| rect.group == group)?
            .rect
            .size,
        DragPayload::FloatingGroup { group } => {
            let floating = controller.layout().floating_group(group)?;
            floating.bounds.size
        }
    };

    let viewport = controller.viewport();
    let min_size = match payload {
        DragPayload::Panel { panel } => controller
            .layout()
            .panel_instance(panel)?
            .constraints
            .min_size,
        DragPayload::Group { .. } | DragPayload::FloatingGroup { .. } => Size::new(1.0, 1.0),
    };
    if viewport.size.width < min_size.width || viewport.size.height < min_size.height {
        return None;
    }

    let size = Size::new(
        preferred.width.min(viewport.size.width).max(min_size.width),
        preferred.height.min(viewport.size.height).max(min_size.height),
    );
    let right = viewport.checked_right().ok()?;
    let bottom = viewport.checked_bottom().ok()?;
    let origin = Point::new(
        (pointer.x - size.width * 0.5).clamp(viewport.origin.x, right - size.width),
        (pointer.y - size.height * 0.5).clamp(viewport.origin.y, bottom - size.height),
    );
    Some(Rect::new(origin, size))
}

fn event_point(x: gpui::Pixels, y: gpui::Pixels) -> Point {
    Point::new(x.as_f32(), y.as_f32())
}

