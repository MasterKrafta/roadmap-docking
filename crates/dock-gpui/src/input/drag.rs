use dock_core::{
    ComputeRectsError, DockCommand, DragDropAction, DragHandle, DragPayload, DragState,
    DropSource, DropTarget, Point, Rect, Size,
};
use gpui::{App, Entity};

use crate::DockController;

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
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => return false,
        };
        let handle = controller.drag().session().handle();

        if let Err(error) = controller
            .drag_mut()
            .session_mut()
            .pointer_move(position)
        {
            tracing::debug!(%error, "dock drag pointer update failed");
            cancel_drag(controller);
            cx.notify();
            return false;
        }

        if !matches!(controller.drag().session().state(), DragState::Dragging { .. }) {
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
        if let Err(error) = controller
            .drag_mut()
            .session_mut()
            .set_drop_target(target)
        {
            tracing::debug!(%error, "could not update dock drag target");
            cancel_drag(controller);
            cx.notify();
            return was_pressed;
        }
        cx.notify();
        was_pressed
    })
}

/// Ends a drag from a host-local release point. It prepares a resolver only when
/// this release itself crosses the Core threshold; otherwise it reuses the
/// resolver captured at that transition. Core revalidates before one command.
pub(crate) fn finish_drag_at(
    controller: &Entity<DockController>,
    position: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, cx| {
        let (payload, was_pressed) = match controller.drag().session().state() {
            DragState::Pressed { payload, .. } => (payload, true),
            DragState::Dragging { payload, .. } => (payload, false),
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                controller.drag_mut().clear_resolver();
                controller.drag_mut().clear_layout_version();
                return false;
            }
        };
        let handle = controller.drag().session().handle();

        if let Err(error) = controller
            .drag_mut()
            .session_mut()
            .pointer_move(position)
        {
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
        if drag_started && handle != Some(DragHandle::FloatingTitle) {
            if let Err(error) = controller
                .drag_mut()
                .session_mut()
                .set_drop_target(target)
            {
                tracing::debug!(%error, "could not set final dock drop target");
                target = None;
            }
        }

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

pub(crate) fn floating_bounds(
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

