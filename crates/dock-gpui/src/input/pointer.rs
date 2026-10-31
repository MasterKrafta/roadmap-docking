use std::{cell::Cell, collections::HashMap, rc::Rc};

use dock_core::{
    DockCommand, DragHandle, DragPayload, DragState, HitResult, Node, NodeId, Point,
};
use gpui::{
    App, DispatchPhase, Entity, HitboxId, MouseButton, MouseDownEvent, MouseExitEvent,
    MouseMoveEvent, MouseUpEvent, ScrollHandle, ScrollWheelEvent, Window,
};

use crate::{
    geometry::host_point_from_gpui_window,
    DockController,
    components::context_menu::{ContextMenuTarget, ContextMenuToggle},
};

use super::{
    drag::{begin_drag_at, finish_drag_at, update_drag_at},
    resize::{begin_resize_hit, finish_resize, update_resize},
    scroll::map_hit_test_point,
};

/// Installs the DockHost-level mouse event pipeline.
///
/// A drag begins as a press. GPUI capture is requested after the Core drag
/// threshold so a normal click still reaches the source control; resize capture
/// is requested immediately on pointer-down. Once captured, move/up continue to
/// the root hitbox outside the source element. Leaving the window or losing
/// capture safely cancels the gesture.
pub(crate) fn register_pointer_events(
    window: &mut Window,
    root_hitbox: HitboxId,
    window_origin: Point,
    controller: Entity<DockController>,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    toggle_context_menu: ContextMenuToggle,
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
) {
    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let toggle_context_menu = toggle_context_menu.clone();
        let pointer_capture = pointer_capture.clone();
        move |event: &MouseDownEvent, phase, window, app| {
            if phase != DispatchPhase::Capture
                || !root_hitbox.is_hovered(window)
                || pointer_capture.get().is_some()
                || window.captured_hitbox().is_some()
            {
                return;
            }

            let Ok(host_point) =
                host_point_from_gpui_window(event.position, window_origin)
            else {
                return;
            };
            let Some(core_point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                return;
            };
            let hit = controller.update(app, |controller, _| controller.hit_test(core_point).ok());

            if event.button == MouseButton::Right {
                let target = match hit {
                    Some(HitResult::Group { group } | HitResult::TabStrip { group }) => {
                        Some(ContextMenuTarget::Group(group))
                    }
                    Some(HitResult::TabHeader { group, panel, .. }) => {
                        Some(ContextMenuTarget::Tab { group, panel })
                    }
                    Some(HitResult::Column { column }) => {
                        Some(ContextMenuTarget::Column(column))
                    }
                    Some(HitResult::TabOverflowButton { group }) => {
                        Some(ContextMenuTarget::Overflow(group))
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    (toggle_context_menu)(target, host_point, app);
                    app.stop_propagation();
                }
                return;
            }

            if event.button != MouseButton::Left {
                return;
            }

            match hit {
                Some(HitResult::TabHeader { panel, group, .. })
                    if tab_header_drag_handle_contains(&controller, group, panel, core_point, app) =>
                {
                    begin_drag_at(
                        &controller,
                        DragPayload::Panel { panel },
                        DragHandle::PanelTab,
                        core_point,
                        app,
                    );
                }
                Some(HitResult::Group { group })
                    if group_grip_contains(&controller, group, core_point, app) =>
                {
                    begin_drag_at(
                        &controller,
                        DragPayload::Group { group },
                        DragHandle::GroupGrip,
                        core_point,
                        app,
                    );
                }
                Some(HitResult::FlyoutPanel { panel, .. })
                    if flyout_title_grip_contains(&controller, core_point, app) =>
                {
                    begin_drag_at(
                        &controller,
                        DragPayload::Panel { panel },
                        DragHandle::FlyoutPanel,
                        core_point,
                        app,
                    );
                }
                Some(HitResult::FloatingTitle { group })
                    if floating_title_grip_contains(&controller, group, core_point, app) =>
                {
                    begin_drag_at(
                        &controller,
                        DragPayload::FloatingGroup { group },
                        DragHandle::FloatingTitle,
                        core_point,
                        app,
                    );
                }
                Some(resize_hit @ (HitResult::Splitter { .. }
                | HitResult::FloatingResizeEdge { .. }
                | HitResult::FlyoutResizeEdge { .. })) => {
                    if let HitResult::FloatingResizeEdge { group, .. } = resize_hit {
                        let _ = controller.update(app, |controller, cx| {
                            let _ = controller.dispatch_in(
                                DockCommand::RaiseFloating { group },
                                cx,
                            );
                        });
                    }
                    begin_resize_hit(&controller, resize_hit, core_point, app);
                    let resize_started = controller
                        .update(app, |controller, _| controller.resize_interaction().is_some());
                    if resize_started {
                        window.capture_pointer(root_hitbox);
                        if window.captured_hitbox() == Some(root_hitbox) {
                            pointer_capture.set(Some(root_hitbox));
                        } else {
                            cancel_pointer_interaction(
                                &controller,
                                window,
                                &pointer_capture,
                                app,
                            );
                            return;
                        }
                        app.stop_propagation();
                    }
                }
                _ => {}
            }
        }
    });

    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let pointer_capture = pointer_capture.clone();
        move |event: &MouseMoveEvent, phase, window, app| {
            if phase != DispatchPhase::Capture {
                return;
            }

            let (drag_active, resize_active) = active_interactions(&controller, app);
            if !drag_active && !resize_active {
                return;
            }
            if !event.dragging() {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            }

            if pointer_capture
                .get()
                .is_some_and(|captured| window.captured_hitbox() != Some(captured))
            {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            }

            let Ok(host_point) =
                host_point_from_gpui_window(event.position, window_origin)
            else {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            };
            let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            };

            if drag_active {
                update_drag_at(&controller, point, app);
            }

            let capture_needed = controller.update(app, |controller, _| {
                matches!(controller.drag().session().state(), DragState::Dragging { .. })
                    || controller.resize_interaction().is_some()
            });
            if capture_needed && pointer_capture.get().is_none() {
                window.capture_pointer(root_hitbox);
                if window.captured_hitbox() == Some(root_hitbox) {
                    pointer_capture.set(Some(root_hitbox));
                } else {
                    cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                    return;
                }
            }

            if resize_active {
                let should_schedule = controller.update(app, |controller, _| {
                    controller.queue_resize_frame_point(point)
                });
                if should_schedule {
                    let controller = controller.clone();
                    window.on_next_frame(move |_, app| {
                        let point = controller.update(app, |controller, _| {
                            controller.take_resize_frame_point()
                        });
                        if let Some(point) = point {
                            update_resize(&controller, point, app);
                        }
                    });
                }
            }
        }
    });

    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let pointer_capture = pointer_capture.clone();
        move |event: &MouseUpEvent, phase, window, app| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }

            let (drag_active, resize_active) = active_interactions(&controller, app);
            if pointer_capture
                .get()
                .is_some_and(|captured| window.captured_hitbox() != Some(captured))
            {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            }
            if !drag_active && !resize_active {
                window.release_pointer();
                pointer_capture.set(None);
                return;
            }

            let Ok(host_point) =
                host_point_from_gpui_window(event.position, window_origin)
            else {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            };
            let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
                return;
            };

            if drag_active {
                finish_drag_at(&controller, point, app);
            }
            if resize_active {
                controller.update(app, |controller, _| {
                    controller.clear_resize_frame_point();
                });
                finish_resize(&controller, point, event.click_count, app);
            }
            window.release_pointer();
            pointer_capture.set(None);
        }
    });

    window.on_mouse_event({
        let controller = controller.clone();
        let pointer_capture = pointer_capture.clone();
        move |_event: &MouseExitEvent, phase, window, app| {
            if phase != DispatchPhase::Capture {
                return;
            }

            let (drag_active, resize_active) = active_interactions(&controller, app);
            if drag_active || resize_active {
                cancel_pointer_interaction(&controller, window, &pointer_capture, app);
            }
        }
    });

    // GPUI's ScrollHandle still applies the native scroll delta. This root
    // listener observes and hit-tests the scroll position without stopping
    // the event, so child scroll containers retain their normal behavior.
    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        move |event: &ScrollWheelEvent, phase, window, app| {
            if phase != DispatchPhase::Capture || !root_hitbox.should_handle_scroll(window) {
                return;
            }
            let Ok(host_point) =
                host_point_from_gpui_window(event.position, window_origin)
            else {
                return;
            };
            if let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            {
                let _ = controller.update(app, |controller, _| controller.hit_test(point));
            }
        }
    });
}

fn core_point_at_host_point(
    point: Point,
    controller: &Entity<DockController>,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    app: &mut App,
) -> Option<Point> {
    controller.update(app, |controller, _| {
        let rects = controller.geometry().ok()?;
        map_hit_test_point(
            point,
            rects,
            scroll_handles,
            controller.metrics().header_height,
        )
        .ok()
    })
}

fn active_interactions(controller: &Entity<DockController>, app: &mut App) -> (bool, bool) {
    controller.update(app, |controller, _| {
        (
            matches!(
                controller.drag().session().state(),
                DragState::Pressed { .. } | DragState::Dragging { .. }
            ),
            controller.resize_interaction().is_some(),
        )
    })
}

fn cancel_pointer_interaction(
    controller: &Entity<DockController>,
    window: &mut Window,
    pointer_capture: &Rc<Cell<Option<HitboxId>>>,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, cx| {
        let result = controller.cancel_pointer_interaction();
        cx.notify();
        result
    }) {
        tracing::debug!(%error, "could not cancel pointer interaction");
    }
    window.release_pointer();
    pointer_capture.set(None);
}

fn tab_header_drag_handle_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    panel: dock_core::PanelId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let Ok(rects) = controller.geometry() else {
            return false;
        };
        let Some(header) = rects.tab_headers.iter().find(|header| {
            header.group == group && header.panel == panel
        }) else {
            return false;
        };

        let is_rail = rects.columns.iter().any(|column_rect| {
            matches!(
                controller.layout().node(column_rect.column),
                Some(Node::Column(column))
                    if column.mode.is_rail() && column.groups.contains(&group)
            )
        });
        if is_rail {
            return true;
        }

        let close_width = 18.0_f32.min(header.rect.size.width.max(0.0));
        point.x < header.rect.origin.x + header.rect.size.width - close_width
    })
}

fn group_grip_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let metrics = controller.metrics();
        let Ok(rects) = controller.geometry() else {
            return false;
        };
        let Some(group_rect) = rects.groups.iter().find(|rect| rect.group == group) else {
            return false;
        };

        let reserved_grip_left = if rects.columns.iter().any(|column_rect| {
            column_rect.rect.origin.y == group_rect.rect.origin.y
                && matches!(
                    controller.layout().node(column_rect.column),
                    Some(Node::Column(column)) if column.groups.first() == Some(&group)
                )
        }) {
            metrics.group_control_button_width + 8.0
        } else {
            0.0
        };
        let control_width = metrics
            .group_control_button_width
            .min(group_rect.rect.size.width / 3.0)
            .max(1.0);
        let header_height = metrics.header_height.min(group_rect.rect.size.height);
        point.y >= group_rect.rect.origin.y
            && point.y < group_rect.rect.origin.y + header_height
            && point.x >= group_rect.rect.origin.x + reserved_grip_left
            && point.x
                < group_rect.rect.origin.x + group_rect.rect.size.width - 2.0 * control_width
    })
}

fn flyout_title_grip_contains(
    controller: &Entity<DockController>,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let metrics = controller.metrics();
        let Ok(rects) = controller.geometry() else {
            return false;
        };
        let Some(flyout) = rects.flyout else {
            return false;
        };
        let header_height = metrics.header_height.min(flyout.rect.size.height);
        let close_width = metrics.group_control_button_width.min(header_height);
        point.y >= flyout.rect.origin.y
            && point.y < flyout.rect.origin.y + header_height
            && point.x < flyout.rect.origin.x + flyout.rect.size.width - close_width
    })
}

fn floating_title_grip_contains(
    controller: &Entity<DockController>,
    group: dock_core::FloatingGroupId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let metrics = controller.metrics();
        let Ok(rects) = controller.geometry() else {
            return false;
        };
        let Some(floating) = rects.floating_groups.iter().find(|rect| rect.group == group) else {
            return false;
        };
        let header_height = metrics.header_height.min(floating.rect.size.height);
        let close_width = metrics.group_control_button_width.min(header_height);
        point.y >= floating.rect.origin.y
            && point.y < floating.rect.origin.y + header_height
            && point.x < floating.rect.origin.x + floating.rect.size.width - close_width
    })
}
