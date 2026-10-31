use dock_core::{
    ComputeRectsError, DragDropAction, DragDropRequest, DragHandle, DragPayload, DragState,
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
                let _ = controller.drag_mut().session_mut().reset();
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
                request.floating_bounds = desired_bounds;
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

/// Calculates the FreeArea preview using the active session's original press point.
/// Keeps the DockHost presentation API independent from DragDropRequest construction.
pub(crate) fn floating_bounds(
    controller: &mut DockController,
    payload: DragPayload,
    pointer: Point,
) -> Option<Rect> {
    let (active_payload, origin) = match controller.drag().session().state() {
        DragState::Dragging {
            payload,
            origin,
            ..
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
        DragPayload::Panel { panel } => controller
            .layout()
            .panel_instance(panel)?
            .constraints
            .preferred_size,
        DragPayload::Group { .. } => source_rect.size,
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
