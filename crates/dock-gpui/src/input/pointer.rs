use std::{cell::Cell, collections::HashMap, rc::Rc};

use dock_core::{
    DockCommand, DragHandle, DragPayload, DragState, FloatingGroupId, HitResult, Layout, Node,
    NodeId, PanelId, Point,
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
    FlyoutPanelCandidate { panel: PanelId },
    FloatingTitleCandidate { group: FloatingGroupId },
    Resize,
    OverflowButton { group: NodeId },
    RaiseFloating { group: FloatingGroupId },
    RaiseFloatingForPanel { panel: PanelId },
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
        HitResult::Splitter { .. }
        | HitResult::FloatingResizeEdge { .. }
        | HitResult::FlyoutResizeEdge { .. } => PointerDownIntent::Resize,
        HitResult::FlyoutPanel { panel, .. } => {
            PointerDownIntent::FlyoutPanelCandidate { panel }
        }
        HitResult::FloatingTitle { group } => {
            PointerDownIntent::FloatingTitleCandidate { group }
        }
        HitResult::FloatingGroup { group } => PointerDownIntent::RaiseFloating { group },
        HitResult::Panel { panel } => PointerDownIntent::RaiseFloatingForPanel { panel },
        HitResult::Canvas => PointerDownIntent::DismissFlyout,
        HitResult::Column { .. } | HitResult::Miss | HitResult::DropZone { .. } => {
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
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
    pointer_press: Rc<Cell<Option<PointerPress>>>,
) {
    window.on_mouse_event({
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let toggle_context_menu = toggle_context_menu.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        move |event: &MouseDownEvent, phase, window, app| {
            if phase != DispatchPhase::Capture
                || !root_hitbox.is_hovered(window)
                || pointer_capture.get().is_some()
                || window.captured_hitbox().is_some()
            {
                return;
            }

            pointer_press.set(None);
            let Ok(host_point) = host_point_from_gpui_window(event.position, window_origin) else {
                return;
            };
            let Some(core_point) =
                core_point_at_host_point(host_point, &controller, &scroll_handles, app)
            else {
                return;
            };
            let hit = controller.update(app, |controller, _| controller.hit(core_point).ok());

            if event.button == MouseButton::Right {
                let target = hit.and_then(context_menu_target_for_hit);
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

            match pointer_down_intent_for_hit(hit) {
                PointerDownIntent::TabHeaderCandidate { group, panel } => {
                    if tab_header_clickable_contains(
                        &controller,
                        group,
                        panel,
                        core_point,
                        app,
                    ) {
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
                    if group_grip_contains(&controller, group, core_point, app) {
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
                PointerDownIntent::FloatingTitleCandidate { group } => {
                    let drag_grip =
                        floating_title_grip_contains(&controller, group, core_point, app);
                    raise_floating_group(&controller, group, app);
                    if drag_grip {
                        begin_drag_at(
                            &controller,
                            DragPayload::FloatingGroup { group },
                            DragHandle::FloatingTitle,
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
                PointerDownIntent::RaiseFloating { group } => {
                    raise_floating_group(&controller, group, app);
                }
                PointerDownIntent::RaiseFloatingForPanel { panel } => {
                    let group = controller.update(app, |controller, _| {
                        floating_group_for_panel(controller.layout(), panel)
                    });
                    if let Some(group) = group {
                        raise_floating_group(&controller, group, app);
                    }
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
        let controller = controller.clone();
        let scroll_handles = scroll_handles.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        move |event: &MouseMoveEvent, phase, window, app| {
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
        let toggle_context_menu = toggle_context_menu.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        move |event: &MouseUpEvent, phase, window, app| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
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

            let drag_was_started = if drag_active {
                finish_drag_at(&controller, point, app)
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

            let intent = pointer_click_intent_for_hit(release_hit, event.click_count);
            if dispatch_pointer_click(intent, &controller, host_point, &toggle_context_menu, app) {
                app.stop_propagation();
            }
        }
    });

    window.on_mouse_event({
        let controller = controller.clone();
        let pointer_capture = pointer_capture.clone();
        let pointer_press = pointer_press.clone();
        move |_event: &MouseExitEvent, phase, window, app| {
            if phase != DispatchPhase::Capture {
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

fn raise_floating_group(
    controller: &Entity<DockController>,
    group: FloatingGroupId,
    app: &mut App,
) {
    let result = controller.update(app, |controller, cx| {
        controller.dispatch_in(DockCommand::RaiseFloating { group }, cx)
    });
    if let Err(error) = result {
        tracing::debug!(%error, "could not raise floating group from pointer-down");
    }
}

fn floating_group_for_panel(layout: &Layout, panel: PanelId) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group, floating)| {
        let root = layout.dock_area(floating.area)?.root;
        subtree_contains_panel(layout, root, panel).then_some(group)
    })
}

fn subtree_contains_panel(layout: &Layout, node: NodeId, panel: PanelId) -> bool {
    match layout.node(node) {
        Some(Node::Panel(candidate)) => candidate.panel_id == panel,
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| subtree_contains_panel(layout, *child, panel)),
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|group| subtree_contains_panel(layout, *group, panel)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| subtree_contains_panel(layout, *child, panel)),
        None => false,
    }
}

fn context_menu_target_for_hit(hit: HitResult) -> Option<ContextMenuTarget> {
    match hit {
        HitResult::Group { group } | HitResult::TabStrip { group } => {
            Some(ContextMenuTarget::Group(group))
        }
        HitResult::TabHeader { group, panel, .. } => {
            Some(ContextMenuTarget::Tab { group, panel })
        }
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
        map_hit_test_point(
            point,
            rects,
            scroll_handles,
            controller.metrics().header_height,
        )
        .ok()
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

fn active_interactions(
    controller: &Entity<DockController>,
    app: &mut App,
) -> (bool, bool, bool) {
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

fn tab_header_clickable_contains(
    controller: &Entity<DockController>,
    group: NodeId,
    panel: PanelId,
    point: Point,
    app: &mut App,
) -> bool {
    controller.update(app, |controller, _| {
        let Ok(rects) = controller.geometry() else {
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


#[cfg(test)]
mod hit_dispatch_tests {
    use dock_core::{
        Axis, FloatingGroupId, HitResult, NodeId, PanelId, ResizeEdge,
    };
    use slotmap::Key;

    use super::{
        PointerClickIntent, PointerDownIntent, PointerPressTarget,
        pointer_click_intent_for_hit, pointer_down_intent_for_hit, pointer_press_target_for_hit,
    };

    #[test]
    fn click_candidates_keep_only_semantic_identity_not_hitresult() {
        let group = NodeId::null();
        let panel = PanelId::null();

        assert_eq!(
            pointer_press_target_for_hit(HitResult::TabHeader { group, panel, index: 0 }),
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
        assert_eq!(
            pointer_press_target_for_hit(HitResult::Canvas),
            None
        );
    }

    #[test]
    fn pointer_down_intents_follow_the_hitresult_table() {
        let group = NodeId::null();
        let panel = PanelId::null();
        let floating = FloatingGroupId::null();

        assert_eq!(
            pointer_down_intent_for_hit(HitResult::TabHeader { group, panel, index: 0 }),
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
            pointer_down_intent_for_hit(HitResult::FloatingTitle { group: floating }),
            PointerDownIntent::FloatingTitleCandidate { group: floating }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::FloatingGroup { group: floating }),
            PointerDownIntent::RaiseFloating { group: floating }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::Panel { panel }),
            PointerDownIntent::RaiseFloatingForPanel { panel }
        );
        assert_eq!(
            pointer_down_intent_for_hit(HitResult::FloatingResizeEdge {
                group: floating,
                edge: ResizeEdge::BottomRight,
            }),
            PointerDownIntent::Resize
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
            PointerDownIntent::None
        );
    }

    #[test]
    fn click_and_double_click_intents_follow_the_hitresult_table() {
        let group = NodeId::null();
        let panel = PanelId::null();

        assert_eq!(
            pointer_click_intent_for_hit(
                HitResult::TabHeader { group, panel, index: 0 },
                1,
            ),
            PointerClickIntent::ActivateTab { group, panel }
        );
        assert_eq!(
            pointer_click_intent_for_hit(
                HitResult::TabHeader { group, panel, index: 0 },
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
                HitResult::TabHeader { group, panel, index: 0 },
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
