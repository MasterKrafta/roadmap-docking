use std::{cell::{Cell, RefCell}, collections::HashMap, rc::Rc};

use dock_core::{
    Axis, DockCommand, DockSide, DragHandle, DragPayload, DragState, HitResult, Node, NodeId,
    PanelId, Point,
};
use gpui::{
    App, DispatchPhase, Entity, HitboxId, MouseButton, MouseDownEvent, MouseExitEvent,
    MouseMoveEvent, MouseUpEvent, ScrollHandle, ScrollWheelEvent, Window,
};

use crate::{
    DockController, FloatingWindowManager,
    components::context_menu::{ContextMenuTarget, ContextMenuToggle},
    dock_components::DockAreaDragArea,
    geometry::host_point_from_gpui_window,
};

use super::{
    drag::{begin_drag_at, finish_drag_at, update_drag_at},
    resize::{begin_resize_hit, finish_resize, update_resize},
    scroll::map_hit_test_point,
};

/// A click candidate derived from one Core hit-test result and retained until
/// pointer-up. The release hit is recomputed; HitResult itself is never cached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PointerPress {
    target: PointerPressTarget,
    layout_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerPressTarget {
    TabHeader { group: NodeId, panel: PanelId },
    TabStrip { group: NodeId },
    OverflowButton { group: NodeId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerDownIntent {
    TabHeaderCandidate { group: NodeId, panel: PanelId },
    GroupCandidate { group: NodeId },
    ColumnCandidate { column: NodeId },
    FlyoutPanelCandidate { panel: PanelId },
    Resize,
    OverflowButton { group: NodeId },
    DismissFlyout,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerClickIntent {
    ActivateTab { group: NodeId, panel: PanelId },
    CollapseGroup { group: NodeId },
    OpenOverflowMenu { group: NodeId },
    None,
}

fn pointer_press_target_for_hit(hit: HitResult) -> Option<PointerPressTarget> {
    match hit {
        HitResult::TabHeader { group, panel, .. } => {
            Some(PointerPressTarget::TabHeader { group, panel })
        }
        HitResult::TabStrip { group } => Some(PointerPressTarget::TabStrip { group }),
        HitResult::TabOverflowButton { group } => {
            Some(PointerPressTarget::OverflowButton { group })
        }
        _ => None,
    }
}

fn pointer_down_intent_for_hit(hit: HitResult) -> PointerDownIntent {
    match hit {
        HitResult::TabHeader { group, panel, .. } => {
            PointerDownIntent::TabHeaderCandidate { group, panel }
        }
        HitResult::TabStrip { group } | HitResult::Group { group } => {
            PointerDownIntent::GroupCandidate { group }
        }
        HitResult::TabOverflowButton { group } => PointerDownIntent::OverflowButton { group },
        HitResult::Column { column } => PointerDownIntent::ColumnCandidate { column },
        HitResult::Splitter { .. }
        | HitResult::FlyoutResizeEdge { .. } => PointerDownIntent::Resize,
        HitResult::FlyoutPanel { panel, .. } => PointerDownIntent::FlyoutPanelCandidate { panel },
        HitResult::Canvas => PointerDownIntent::DismissFlyout,
        HitResult::FloatingGroup { .. }
        | HitResult::Panel { .. }
        | HitResult::Miss
        | HitResult::DropZone { .. } => {
            PointerDownIntent::None
        }
    }
}

fn pointer_click_intent_for_hit(hit: HitResult, click_count: usize) -> PointerClickIntent {
    match hit {
        HitResult::TabHeader { group, .. } if click_count == 2 => {
            PointerClickIntent::CollapseGroup { group }
        }
        HitResult::TabHeader { group, panel, .. } if click_count == 1 => {
            PointerClickIntent::ActivateTab { group, panel }
        }
        HitResult::TabStrip { group } if click_count == 2 => {
            PointerClickIntent::CollapseGroup { group }
        }
        HitResult::TabOverflowButton { group } if click_count == 1 => {
            PointerClickIntent::OpenOverflowMenu { group }
        }
        _ => PointerClickIntent::None,
    }
}

/// Shared pointer state retained by DockHost across rerenders.
pub(crate) struct PointerEventState {
    pub(crate) manager: Entity<FloatingWindowManager>,
    pub(crate) pointer_capture: Rc<Cell<Option<HitboxId>>>,
    pub(crate) pointer_press: Rc<Cell<Option<PointerPress>>>,
}

/// Installs the DockHost-level mouse event pipeline. Core HitResult is the
/// only source of dock gesture intent; components keep only explicit button
/// actions that do not have a HitResult representation.
pub(crate) fn register_pointer_events(
    window: &mut Window,
    root_hitbox: HitboxId,
    window_origin: Point,
    controller: Entity<DockController>,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    toggle_context_menu: ContextMenuToggle,
    pointer_state: PointerEventState,
) {
    let PointerEventState {
        manager,
        pointer_capture,
        pointer_press,
    } = pointer_state;
    // Keep console output focused: during a drag we print only when the
    // Core hit-test result changes under the pointer.
    let last_drag_hit = Rc::new(RefCell::new(None::<String>));

    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let toggle_context_menu = toggle_context_menu.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        let last_drag_hit_for_down = last_drag_hit.clone();
        move |event: &MouseDownEvent, phase, window, app| {
            if phase != DispatchPhase::Capture
                || !root_hitbox.is_hovered(window)
                || pointer_capture.get().is_some()
                || window.captured_hitbox().is_some()
            {
                return;
            }

            pointer_press.set(None);
            *last_drag_hit_for_down.borrow_mut() = None;
            let Ok(host_point) = host_point_from_gpui_window(event.position, window_origin) else {
                return;
            };
            let Some(core_point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                return;
            };
            let hit = controller.update(app, |controller, _| controller.hit(core_point).ok());
            let hit_label = hit
                .as_ref()
                .map(|hit| format!("{hit:?}"))
                .unwrap_or_else(|| "Miss".to_owned());
            eprintln!(
                "[DOCK-INPUT][main] down button={:?} host=({:.1}, {:.1}) core=({:.1}, {:.1}) hit={}",
                event.button, host_point.x, host_point.y, core_point.x, core_point.y, hit_label
            );

            if event.button == MouseButton::Right {
                let target = hit
                    .filter(|hit| !dock_component_interactions_disabled_for_hit(
                        &controller,
                        *hit,
                        app,
                    ))
                    .and_then(context_menu_target_for_hit);
                if let Some(target) = target {
                    (toggle_context_menu)(target, host_point, app);
                    app.stop_propagation();
                }
                return;
            }

            if event.button != MouseButton::Left {
                return;
            }
            let Some(hit) = hit else {
                return;
            };
            eprintln!(
                "[DOCK-INPUT][main] down_intent={:?}",
                pointer_down_intent_for_hit(hit)
            );

            match pointer_down_intent_for_hit(hit) {
                PointerDownIntent::TabHeaderCandidate { group, panel } => {
                    let tab_is_clickable = tab_header_clickable_contains(
                        &controller,
                        group,
                        panel,
                        core_point,
                        app,
                    );
                    if tab_is_clickable {
                        let layout_version =
                            controller.update(app, |controller, _| controller.layout().version());
                        pointer_press.set(Some(PointerPress {
                            target: PointerPressTarget::TabHeader { group, panel },
                            layout_version,
                        }));
                    }
                    if tab_header_drag_handle_contains(
                        &controller,
                        group,
                        panel,
                        core_point,
                        app,
                    ) {
                        begin_drag_at(
                            &controller,
                            DragPayload::Panel { panel },
                            DragHandle::PanelTab,
                            core_point,
                            app,
                        );
                    }
                }
                PointerDownIntent::GroupCandidate { group } => {
                    if group_grip_contains(&controller, group, core_point, app)
                    {
                        if matches!(hit, HitResult::TabStrip { .. }) {
                            let layout_version = controller
                                .update(app, |controller, _| controller.layout().version());
                            pointer_press.set(Some(PointerPress {
                                target: PointerPressTarget::TabStrip { group },
                                layout_version,
                            }));
                        }
                        begin_drag_at(
                            &controller,
                            DragPayload::Group { group },
                            DragHandle::GroupGrip,
                            core_point,
                            app,
                        );
                    }
                }
                PointerDownIntent::ColumnCandidate { column } => {
                    if dock_area_drag_handle_contains(
                        &controller,
                        column,
                        core_point,
                        app,
                    ) {
                        begin_drag_at(
                            &controller,
                            DragPayload::Column { column },
                            DragHandle::ColumnHeader,
                            core_point,
                            app,
                        );
                    }
                }
                PointerDownIntent::FlyoutPanelCandidate { panel } => {
                    if flyout_title_grip_contains(&controller, core_point, app) {
                        begin_drag_at(
                            &controller,
                            DragPayload::Panel { panel },
                            DragHandle::FlyoutPanel,
                            core_point,
                            app,
                        );
                    }
                }
                PointerDownIntent::Resize => {
                    begin_resize_hit(&controller, hit, core_point, app);
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
                                &pointer_press,
                                app,
                            );
                            return;
                        }
                        app.stop_propagation();
                    }
                }
                PointerDownIntent::OverflowButton { group } => {
                    let layout_version =
                        controller.update(app, |controller, _| controller.layout().version());
                    pointer_press.set(Some(PointerPress {
                        target: PointerPressTarget::OverflowButton { group },
                        layout_version,
                    }));
                }
                PointerDownIntent::DismissFlyout => {
                    let flyout_is_open =
                        controller.update(app, |controller, _| controller.layout().flyout().is_some());
                    if flyout_is_open {
                        let result = controller.update(app, |controller, cx| {
                            controller.dispatch_transient_in(DockCommand::DismissFlyout, cx)
                        });
                        if let Err(error) = result {
                            tracing::debug!(%error, "could not dismiss flyout from Canvas pointer-down");
                        }
                    }
                }
                PointerDownIntent::None => {}
            }
        }
    });

    window.on_mouse_event({
        let manager = manager.clone();
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        let last_drag_hit_for_move = last_drag_hit.clone();
        move |event: &MouseMoveEvent, phase, window, app| {
            if manager.update(app, |manager, _| manager.native_move_active()) {
                if pointer_capture.get().is_some() {
                    window.release_pointer();
                    pointer_capture.set(None);
                    pointer_press.set(None);
                }
                return;
            }
            if phase != DispatchPhase::Capture {
                return;
            }

            if !ensure_pointer_interaction_current(&controller, window, &pointer_capture, app) {
                pointer_press.set(None);
                return;
            }
            let (drag_active, _, resize_active) = active_interactions(&controller, app);
            if !drag_active && !resize_active {
                return;
            }
            if !event.dragging() {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            }

            if pointer_capture
                .get()
                .is_some_and(|captured| window.captured_hitbox() != Some(captured))
            {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            }

            let Ok(host_point) = host_point_from_gpui_window(event.position, window_origin) else {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            };
            let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            };

            let drag_just_started = drag_active && update_drag_at(&controller, point, app);
            if drag_just_started {
                // Once Core crosses the drag threshold, this press can no
                // longer be dispatched as a tab click if the gesture is cancelled.
                pointer_press.set(None);
            }

            let drag_details = controller.update(app, |controller, _| {
                match controller.drag().session().state() {
                    DragState::Dragging { payload, origin, .. } => {
                        Some((payload, controller.drag().session().handle(), origin))
                    }
                    _ => None,
                }
            });
            if let Some((payload, handle, origin)) = drag_details {
                let hit_label = controller.update(app, |controller, _| {
                    controller
                        .hit(point)
                        .map(|hit| format!("{hit:?}"))
                        .unwrap_or_else(|_| "Miss".to_owned())
                });
                let mut previous_hit = last_drag_hit_for_move.borrow_mut();
                if previous_hit.as_deref() != Some(hit_label.as_str()) {
                    let phase_label = if drag_just_started { "drag_start" } else { "drag_over" };
                    eprintln!(
                        "[DOCK-INPUT][main] {} payload={:?} handle={:?} origin=({:.1}, {:.1}) pointer=({:.1}, {:.1}) hit={}",
                        phase_label, payload, handle, origin.x, origin.y, point.x, point.y, hit_label
                    );
                    *previous_hit = Some(hit_label);
                }
            }

            let should_tearoff = controller.update(app, |controller, _| {
                native_tearoff_required(controller, point)
            });
            if should_tearoff
                && super::drag::begin_native_tearoff(&controller, &manager, point, app)
            {
                window.release_pointer();
                pointer_capture.set(None);
                pointer_press.set(None);
                app.stop_propagation();
                return;
            }

            let capture_needed = controller.update(app, |controller, _| {
                matches!(
                    controller.drag().session().state(),
                    DragState::Dragging { .. }
                ) || controller.resize_interaction().is_some()
            });
            if capture_needed && pointer_capture.get().is_none() {
                window.capture_pointer(root_hitbox);
                if window.captured_hitbox() == Some(root_hitbox) {
                    pointer_capture.set(Some(root_hitbox));
                } else {
                    cancel_pointer_interaction(
                        &controller,
                        window,
                        &pointer_capture,
                        &pointer_press,
                        app,
                    );
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
                        let point = controller
                            .update(app, |controller, _| controller.take_resize_frame_point());
                        if let Some(point) = point {
                            update_resize(&controller, point, app);
                        }
                    });
                }
            }
        }
    });

    window.on_mouse_event({
        let manager = manager.clone();
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let toggle_context_menu = toggle_context_menu.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        let last_drag_hit_for_up = last_drag_hit.clone();
        move |event: &MouseUpEvent, phase, window, app| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            if manager.update(app, |manager, _| manager.native_move_active()) {
                window.release_pointer();
                pointer_capture.set(None);
                pointer_press.set(None);
                return;
            }
            if !ensure_pointer_interaction_current(&controller, window, &pointer_capture, app) {
                pointer_press.set(None);
                return;
            }
            let (drag_active, _, resize_active) = active_interactions(&controller, app);
            if pointer_capture
                .get()
                .is_some_and(|captured| window.captured_hitbox() != Some(captured))
            {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            }

            let pending_press = pointer_press.get();
            if !drag_active && !resize_active && pending_press.is_none() {
                window.release_pointer();
                pointer_capture.set(None);
                return;
            }

            let Ok(host_point) = host_point_from_gpui_window(event.position, window_origin) else {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            };
            let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
                return;
            };
            let release_hit = controller.update(app, |controller, _| {
                controller
                    .hit(point)
                    .map(|hit| format!("{hit:?}"))
                    .unwrap_or_else(|_| "Miss".to_owned())
            });
            eprintln!(
                "[DOCK-INPUT][main] up host=({:.1}, {:.1}) core=({:.1}, {:.1}) drag_active={} resize_active={} hit={}",
                host_point.x, host_point.y, point.x, point.y, drag_active, resize_active, release_hit
            );

            let drag_was_started = if drag_active {
                // MouseUp can be the first event delivered beyond the drag threshold.
                // Update the session once more so a tab released outside its strip
                // still becomes a floating window instead of being treated as a failed drop.
                if update_drag_at(&controller, point, app) {
                    pointer_press.set(None);
                }
                let should_tearoff = controller.update(app, |controller, _| {
                    native_tearoff_required(controller, point)
                });
                if should_tearoff
                    && super::drag::begin_native_tearoff(&controller, &manager, point, app)
                {
                    window.release_pointer();
                    pointer_capture.set(None);
                    pointer_press.set(None);
                    app.stop_propagation();
                    return;
                }
                finish_drag_at(&controller, &manager, point, app)
            } else {
                false
            };
            if resize_active {
                controller.update(app, |controller, _| {
                    controller.clear_resize_frame_point();
                });
                finish_resize(&controller, point, event.click_count, app);
            }
            window.release_pointer();
            pointer_capture.set(None);

            pointer_press.set(None);
            *last_drag_hit_for_up.borrow_mut() = None;
            if drag_was_started || resize_active {
                return;
            }

            let Some(press) = pending_press else {
                return;
            };
            let (layout_version, release_hit) = controller.update(app, |controller, _| {
                (controller.layout().version(), controller.hit(point).ok())
            });
            let Some(release_hit) = release_hit else {
                return;
            };
            if layout_version != press.layout_version
                || pointer_press_target_for_hit(release_hit) != Some(press.target)
            {
                return;
            }

            let disable_component_actions = dock_component_interactions_disabled_for_hit(
                &controller,
                release_hit,
                app,
            );
            let intent = pointer_click_intent_for_hit(release_hit, event.click_count);
            eprintln!(
                "[DOCK-INPUT][main] click_dispatch click_count={} hit={:?} intent={:?}",
                event.click_count, release_hit, intent
            );
            let intent = if disable_component_actions
                && matches!(intent, PointerClickIntent::CollapseGroup { .. })
            {
                PointerClickIntent::None
            } else {
                intent
            };
            if dispatch_pointer_click(intent, &controller, host_point, &toggle_context_menu, app) {
                app.stop_propagation();
            }
        }
    });

    window.on_mouse_event({
        let manager = manager.clone();
        let controller = controller.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        move |_event: &MouseExitEvent, phase, window, app| {
            if phase != DispatchPhase::Capture {
                return;
            }
            if manager.update(app, |manager, _| manager.native_move_active()) {
                window.release_pointer();
                pointer_capture.set(None);
                pointer_press.set(None);
                return;
            }
            pointer_press.set(None);

            let (drag_active, _, resize_active) = active_interactions(&controller, app);
            if drag_active || resize_active || pointer_capture.get().is_some() {
                cancel_pointer_interaction(
                    &controller,
                    window,
                    &pointer_capture,
                    &pointer_press,
                    app,
                );
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
            let Ok(host_point) = host_point_from_gpui_window(event.position, window_origin) else {
                return;
            };
            if let Some(point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            {
                let _ = controller.update(app, |controller, _| controller.hit(point));
            }
        }
    });
}

/// Selects which semantic drag areas may start a native tear-off.
/// DockTab in the new Column renderer routes as a Column drag; legacy PanelTab
/// gestures become native windows after leaving their source TabStripRect.
fn native_tearoff_enabled_for(handle: Option<DragHandle>, payload: DragPayload) -> bool {
    match (handle, payload) {
        (Some(DragHandle::ColumnHeader), DragPayload::Column { .. }) => true,
        (Some(DragHandle::PanelTab), DragPayload::Panel { .. }) => true,
        (Some(DragHandle::GroupGrip), DragPayload::Group { .. }) => false,
        _ => false,
    }
}

/// Checks whether the active gesture has crossed its native tear-off boundary.
fn native_tearoff_required(controller: &mut DockController, point: Point) -> bool {
    let DragState::Dragging { payload, .. } = controller.drag().session().state() else {
        return false;
    };
    let handle = controller.drag().session().handle();
    native_tearoff_enabled_for(handle, payload)
        && native_tearoff_boundary_reached(controller, handle, payload, point)
}

/// Checks the semantic boundary that turns an in-layout drag into a native tear-off.
fn native_tearoff_boundary_reached(
    controller: &mut DockController,
    handle: Option<DragHandle>,
    payload: DragPayload,
    point: Point,
) -> bool {
    match (handle, payload) {
        (Some(DragHandle::ColumnHeader), DragPayload::Column { .. }) => true,
        (Some(DragHandle::PanelTab), DragPayload::Panel { panel }) => {
            // Resolve the group first, then release the mutable geometry borrow before
            // consulting the layout to determine which renderer owns its header.
            let group = {
                let Ok(rects) = controller.geometry() else {
                    return false;
                };
                let Some(group) = rects
                    .tab_headers
                    .iter()
                    .find(|header| header.panel == panel)
                    .map(|header| header.group)
                else {
                    return false;
                };
                group
            };
            let uses_dock_component = is_dock_component_tab_group(controller.layout(), group);
            let Ok(rects) = controller.geometry() else {
                return false;
            };
            let drag_zone = if uses_dock_component {
                let Some(group_rect) = rects.groups.iter().find(|rect| rect.group == group) else {
                    return false;
                };
                DockTab::drag_zone(group_rect.rect, controller.metrics())
            } else {
                let Some(strip) = rects
                    .tab_strips
                    .iter()
                    .find(|strip| strip.group == group)
                    .map(|strip| strip.rect)
                else {
                    return false;
                };
                strip
            };
            DockTab::leaves_strip_zone(drag_zone, point)
        }
        _ => false,
    }
}

fn dispatch_pointer_click(
    intent: PointerClickIntent,
    controller: &Entity<DockController>,
    anchor: Point,
    toggle_context_menu: &ContextMenuToggle,
    app: &mut App,
) -> bool {
    match intent {
        PointerClickIntent::ActivateTab { group, panel } => {
            let result = controller.update(app, |controller, cx| {
                controller
                    .dispatch_in(DockCommand::ActivateTab { group, panel }, cx)
                    .map(|_| ())
            });
            if let Err(error) = result {
                tracing::debug!(%error, "could not activate tab from pointer click");
            }
            true
        }
        PointerClickIntent::CollapseGroup { group } => {
            let result = controller.update(app, |controller, cx| {
                let collapsed = match controller.layout().node(group) {
                    Some(Node::Tabs(tabs)) => !tabs.collapsed,
                    _ => return Ok(()),
                };
                controller
                    .dispatch_in(DockCommand::CollapseGroup { group, collapsed }, cx)
                    .map(|_| ())
            });
            if let Err(error) = result {
                tracing::debug!(%error, "could not toggle collapsed group from double-click");
            }
            true
        }
        PointerClickIntent::OpenOverflowMenu { group } => {
            (toggle_context_menu)(ContextMenuTarget::Overflow(group), anchor, app);
            true
        }
        PointerClickIntent::None => false,
    }
}

fn context_menu_target_for_hit(hit: HitResult) -> Option<ContextMenuTarget> {
    match hit {
        HitResult::Group { group } | HitResult::TabStrip { group } => {
            Some(ContextMenuTarget::Group(group))
        }
        HitResult::TabHeader { group, panel, .. } => Some(ContextMenuTarget::Tab { group, panel }),
        HitResult::Column { column } => Some(ContextMenuTarget::Column(column)),
        HitResult::TabOverflowButton { group } => Some(ContextMenuTarget::Overflow(group)),
        _ => None,
    }
}

fn core_point_at_host_point(
    point: Point,
    controller: &Entity<DockController>,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    app: &mut App,
) -> Option<Point> {
    controller.update(app, |controller, _| {
        let rects = controller.geometry().ok()?;
        map_hit_test_point(point, rects, scroll_handles).ok()
    })
}

fn ensure_pointer_interaction_current(
    controller: &Entity<DockController>,
    window: &mut Window,
    pointer_capture: &Rc<Cell<Option<HitboxId>>>,
    app: &mut App,
) -> bool {
    let (current, active) = controller.update(app, |controller, cx| {
        let current = controller.ensure_pointer_interaction_current();
        if !current {
            cx.notify();
        }
        (current, controller.has_active_pointer_interaction())
    });
    if !current || (!active && pointer_capture.get().is_some()) {
        window.release_pointer();
        pointer_capture.set(None);
    }
    current
}

fn active_interactions(controller: &Entity<DockController>, app: &mut App) -> (bool, bool, bool) {
    controller.update(app, |controller, _| {
        let state = controller.drag().session().state();
        (
            matches!(
                state,
                DragState::Pressed { .. } | DragState::Dragging { .. }
            ),
            matches!(state, DragState::Dragging { .. }),
            controller.resize_interaction().is_some(),
        )
    })
}

fn cancel_pointer_interaction(
    controller: &Entity<DockController>,
    window: &mut Window,
    pointer_capture: &Rc<Cell<Option<HitboxId>>>,
    pointer_press: &Rc<Cell<Option<PointerPress>>>,
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
    pointer_press.set(None);
}

fn dock_component_interactions_disabled_for_hit(
    controller: &Entity<DockController>,
    hit: HitResult,
    app: &mut App,
) -> bool {
    match hit {
        HitResult::Column { column } => uses_dock_component_column(controller, column, app),
        HitResult::Group { group }
        | HitResult::TabStrip { group }
        | HitResult::TabHeader { group, .. } => {
            uses_dock_component_group(controller, group, app)
        }
        _ => false,
    }
}

fn uses_dock_component_group(
    controller: &Entity<DockController>,
    group_id: NodeId,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        is_dock_component_tab_group(controller.layout(), group_id)
    })
}

fn is_dock_component_tab_group(layout: &dock_core::Layout, group_id: NodeId) -> bool {
    let is_single_panel_group = matches!(
        layout.node(group_id),
        Some(Node::Tabs(tabs))
            if tabs.children.len() == 1
                && tabs.children.first().is_some_and(
                    |child| matches!(layout.node(*child), Some(Node::Panel(_)))
                )
    );
    if !is_single_panel_group {
        return false;
    }

    layout.nodes().any(|(column_id, node)| {
        let Node::Column(column) = node else {
            return false;
        };
        column.groups.len() == 1
            && column.groups.first() == Some(&group_id)
            && is_main_dock_column(layout, column_id)
    })
}

fn uses_dock_component_column(
    controller: &Entity<DockController>,
    column_id: NodeId,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let layout = controller.layout();
        let Some(Node::Column(column)) = layout.node(column_id) else {
            return false;
        };
        let Some(group_id) = column.groups.first().copied() else {
            return false;
        };
        column.groups.len() == 1
            && is_main_dock_column(layout, column_id)
            && matches!(
                layout.node(group_id),
                Some(Node::Tabs(tabs))
                    if tabs.children.len() == 1
                        && tabs.children.first().is_some_and(
                            |child| matches!(layout.node(*child), Some(Node::Panel(_)))
                        )
            )
    })
}

fn is_main_dock_column(layout: &dock_core::Layout, column: NodeId) -> bool {
    [DockSide::Left, DockSide::Right].into_iter().any(|side| {
        let Some(area) = layout.main_area(side) else {
            return false;
        };
        match layout.node(area.root) {
            Some(Node::Column(_)) => area.root == column,
            Some(Node::Split(split)) => {
                split.axis == Axis::X && split.children.contains(&column)
            }
            _ => false,
        }
    })
}

fn tab_header_clickable_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    panel: PanelId,
    point: Point,
    app: &mut App,
) -> bool {
    let uses_component_tab = uses_dock_component_group(controller, group, app);
    controller.update(app, |controller, _| {
        let Ok(rects) = controller.geometry().cloned() else {
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
            return false;
        }
        let Some(header) = rects
            .tab_headers
            .iter()
            .find(|header| header.group == group && header.panel == panel)
        else {
            return false;
        };
        if uses_component_tab {
            return matches!(header.rect.contains(point), Ok(true));
        }
        let close_width = 18.0_f32.min(header.rect.size.width.max(0.0));
        point.x < header.rect.origin.x + header.rect.size.width - close_width
    })
}

fn tab_header_drag_handle_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    panel: dock_core::PanelId,
    point: Point,
    app: &mut App,
) -> bool {
    let uses_component_tab = uses_dock_component_group(controller, group, app);
    controller.update(app, |controller, _| {
        let Ok(rects) = controller.geometry().cloned() else {
            return false;
        };
        let Some(header) = rects
            .tab_headers
            .iter()
            .find(|header| header.group == group && header.panel == panel)
        else {
            return false;
        };

        let is_rail = rects.columns.iter().any(|column_rect| {
            matches!(
                controller.layout().node(column_rect.column),
                Some(Node::Column(column))
                    if column.mode.is_rail() && column.groups.contains(&group)
            )
        });
        if is_rail || uses_component_tab {
            return matches!(header.rect.contains(point), Ok(true));
        }

        let close_width = 18.0_f32.min(header.rect.size.width.max(0.0));
        point.x < header.rect.origin.x + header.rect.size.width - close_width
    })
}

fn dock_area_drag_handle_contains(
    controller: &Entity<DockController>,
    column: NodeId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let metrics = controller.metrics();
        let Ok(rects) = controller.geometry().cloned() else {
            return false;
        };
        let layout = controller.layout();

        // Core currently starts a Column drag only when the DockArea contains
        // exactly one Column. Do not detach a single child from a multi-Column area.
        for side in [DockSide::Left, DockSide::Right] {
            let Some(area) = layout.main_area(side) else {
                continue;
            };
            let mut area_column_ids = Vec::new();
            collect_dock_area_columns(layout, area.root, &mut area_column_ids);
            if area_column_ids.len() != 1 || area_column_ids[0] != column {
                continue;
            }
            let Some(column_rect) = rects.columns.iter().find(|rect| rect.column == column) else {
                return false;
            };
            let area_columns = [*column_rect];
            return DockAreaDragArea::contains(
                column_rect.rect,
                &area_columns,
                point,
                metrics.column_header_height,
                metrics.column_control_button_width,
            );
        }

        false
    })
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

fn group_grip_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let metrics = controller.metrics();
        let Ok(rects) = controller.geometry().cloned() else {
            return false;
        };
        let Some(group_rect) = rects.groups.iter().find(|rect| rect.group == group) else {
            return false;
        };

        let is_column_group = rects.columns.iter().any(|column_rect| {
            matches!(
                controller.layout().node(column_rect.column),
                Some(Node::Column(column)) if column.groups.contains(&group)
            )
        });
        let reserved_grip_left = if is_column_group {
            rects
                .tab_headers
                .iter()
                .filter(|header| header.group == group)
                .map(|header| header.rect.origin.x + header.rect.size.width)
                .fold(group_rect.rect.origin.x, f32::max)
                - group_rect.rect.origin.x
        } else if rects.columns.iter().any(|column_rect| {
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

        let requested_header_height = if is_column_group {
            metrics.header_height.max(metrics.tab_strip_height)
        } else {
            metrics.header_height
        };
        let header_height = requested_header_height.min(group_rect.rect.size.height);
        let right_edge = group_rect.rect.origin.x + group_rect.rect.size.width;
        let right_reserve =
            (metrics.group_grip_width + metrics.group_control_button_width)
                .min(group_rect.rect.size.width);

        point.y >= group_rect.rect.origin.y
            && point.y < group_rect.rect.origin.y + header_height
            && point.x >= group_rect.rect.origin.x + reserved_grip_left
            && point.x < right_edge - right_reserve
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

#[cfg(test)]
mod hit_dispatch_tests {
    use dock_core::{
        Axis, DragHandle, DragPayload, FloatingGroupId, HitResult, NodeId, PanelId, Point, Rect,
        ResizeEdge,
    };
    use slotmap::Key;

    use super::{
        PointerClickIntent, PointerDownIntent, PointerPressTarget, native_tearoff_enabled_for,
        pointer_click_intent_for_hit, pointer_down_intent_for_hit, pointer_press_target_for_hit,
    };

    #[test]
    fn click_candidates_keep_only_semantic_identity_not_hitresult() {
        let group = NodeId::null();
        let panel = PanelId::null();

        assert_eq!(
            pointer_press_target_for_hit(HitResult::TabHeader {
                group,
                panel,
                index: 0
            }),
            Some(PointerPressTarget::TabHeader { group, panel })
        );
        assert_eq!(
            pointer_press_target_for_hit(HitResult::TabStrip { group }),
            Some(PointerPressTarget::TabStrip { group })
        );
        assert_eq!(
            pointer_press_target_for_hit(HitResult::TabOverflowButton { group }),
            Some(PointerPressTarget::OverflowButton { group })
        );
        assert_eq!(pointer_press_target_for_hit(HitResult::Canvas), None);
    }

    #[test]
    fn pointer_down_intents_follow_the_hitresult_table() {
        let group = NodeId::null();
        let panel = PanelId::null();
        let floating = FloatingGroupId::null();

        assert_eq!(
            pointer_down_intent_for_hit(HitResult::TabHeader {
                group,
                panel,
                index: 0
            }),
            PointerDownIntent::TabHeaderCandidate { group, panel }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::TabStrip { group }),
            PointerDownIntent::GroupCandidate { group }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::TabOverflowButton { group }),
            PointerDownIntent::OverflowButton { group }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Group { group }),
            PointerDownIntent::GroupCandidate { group }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Splitter {
                owner: group,
                boundary_index: 0,
                axis: Axis::Y,
            }),
            PointerDownIntent::Resize
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::FlyoutPanel {
                column: group,
                group,
                panel,
            }),
            PointerDownIntent::FlyoutPanelCandidate { panel }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::FlyoutResizeEdge {
                column: group,
                panel,
                edge: ResizeEdge::Left,
            }),
            PointerDownIntent::Resize
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::FloatingGroup { group: floating }),
            PointerDownIntent::None
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Panel { panel }),
            PointerDownIntent::None
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Canvas),
            PointerDownIntent::DismissFlyout
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Miss),
            PointerDownIntent::None
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Column { column: group }),
            PointerDownIntent::ColumnCandidate { column: group }
        );
    }

    #[test]
    fn native_tearoff_routing_enables_columns_and_individual_tabs() {
        let column = NodeId::null();
        let group = NodeId::null();
        let panel = PanelId::null();

        assert!(native_tearoff_enabled_for(
            Some(DragHandle::ColumnHeader),
            DragPayload::Column { column },
        ));
        assert!(native_tearoff_enabled_for(
            Some(DragHandle::PanelTab),
            DragPayload::Panel { panel },
        ));
        assert!(!native_tearoff_enabled_for(
            Some(DragHandle::GroupGrip),
            DragPayload::Group { group },
        ));
        assert!(!native_tearoff_enabled_for(
            Some(DragHandle::ColumnHeader),
            DragPayload::Group { group },
        ));
    }

    #[test]
    fn dock_area_header_drag_zone_excludes_collapse_and_close_controls() {
        let rect = Rect::new(
            Point::new(10.0, 20.0),
            dock_core::Size::new(240.0, 500.0),
        );

        let columns = [dock_core::ColumnRect::new(NodeId::null(), rect)];

        assert!(crate::dock_components::DockAreaDragArea::contains(
            rect,
            &columns,
            Point::new(100.0, 30.0),
            32.0,
            28.0,
        ));
        assert!(!crate::dock_components::DockAreaDragArea::contains(
            rect,
            &columns,
            Point::new(200.0, 30.0),
            32.0,
            28.0,
        ));
        assert!(!crate::dock_components::DockAreaDragArea::contains(
            rect,
            &columns,
            Point::new(240.0, 30.0),
            32.0,
            28.0,
        ));
        assert!(!crate::dock_components::DockAreaDragArea::contains(
            rect,
            &columns,
            Point::new(100.0, 52.0),
            32.0,
            28.0,
        ));
    }

    #[test]
    fn click_and_double_click_intents_follow_the_hitresult_table() {
        let group = NodeId::null();
        let panel = PanelId::null();

        assert_eq!(
            pointer_click_intent_for_hit(
                HitResult::TabHeader {
                    group,
                    panel,
                    index: 0
                },
                1,
            ),
            PointerClickIntent::ActivateTab { group, panel }
        );
        assert_eq!(
            pointer_click_intent_for_hit(
                HitResult::TabHeader {
                    group,
                    panel,
                    index: 0
                },
                2,
            ),
            PointerClickIntent::CollapseGroup { group }
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::TabStrip { group }, 1),
            PointerClickIntent::None
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::TabStrip { group }, 2),
            PointerClickIntent::CollapseGroup { group }
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::TabOverflowButton { group }, 1),
            PointerClickIntent::OpenOverflowMenu { group }
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::TabOverflowButton { group }, 2),
            PointerClickIntent::None
        );
        assert_eq!(
            pointer_click_intent_for_hit(
                HitResult::TabHeader {
                    group,
                    panel,
                    index: 0
                },
                3,
            ),
            PointerClickIntent::None
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::Group { group }, 2),
            PointerClickIntent::None
        );
        assert_eq!(
            pointer_click_intent_for_hit(HitResult::Canvas, 1),
            PointerClickIntent::None
        );
    }
}
