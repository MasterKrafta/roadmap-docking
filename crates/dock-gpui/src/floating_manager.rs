//! Runtime owner of the native windows corresponding to Core floating groups.
//!
//! The manager stores only window handles and derived panel ownership. The
//! shared DockController remains the sole owner of Layout and panel views.

use std::{collections::{HashMap, HashSet}, time::Duration};

use dock_core::{
    DockCommand, DockSide, DragPayload, DragState, DropSource, DropTarget, FloatingGroupId,
    PanelId, Point, Rect, Size,
};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, Window, WindowBounds,
    WindowHandle, WindowId, WindowKind, WindowOptions, point, px, size,
};
use platform::{
    NativeKey, NativeWindowId, NativeWindowOps, PhysicalPoint, PhysicalSize, SystemNativeWindowOps,
};

use crate::{
    DockController, DockEvent, DockTheme,
    components::FloatingHost,
    dock_host::collect_panel_ids,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelWindowOwner {
    Main,
    Floating(FloatingGroupId),
}

struct FloatingWindowEntry {
    window: WindowHandle<FloatingHost>,
    window_id: WindowId,
    native_window: Option<NativeWindowId>,
    bounds: Rect,
    z_order: i32,
    native_min_size: Option<PhysicalSize>,
    native_scale_factor: Option<f32>,
    resize_enabled: bool,
}

/// Maps Core floating groups to their native GPUI windows.
///
/// Windows released after a group merge are hidden and retained for the next
/// extraction. This is a small reuse cache, not a pre-created window pool:
/// entries are added only when a real floating composition disappears.
#[derive(Default)]
pub struct WindowRegistry {
    floating: HashMap<FloatingGroupId, FloatingWindowEntry>,
    reusable: Vec<FloatingWindowEntry>,
    panel_owners: HashMap<PanelId, PanelWindowOwner>,
}

impl WindowRegistry {
    pub fn floating_count(&self) -> usize {
        self.floating.len()
    }

    pub fn reusable_count(&self) -> usize {
        self.reusable.len()
    }

    pub fn window_for_group(
        &self,
        group: FloatingGroupId,
    ) -> Option<WindowHandle<FloatingHost>> {
        self.floating.get(&group).map(|entry| entry.window)
    }

    pub fn native_window_for_group(
        &self,
        group: FloatingGroupId,
    ) -> Option<NativeWindowId> {
        self.floating.get(&group)?.native_window
    }

    pub fn owner_for_panel(&self, panel: PanelId) -> Option<PanelWindowOwner> {
        self.panel_owners.get(&panel).copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeMoveKind {
    TearOff,
    ExistingWindow,
}

#[derive(Clone, Copy)]
struct NativeMoveTarget {
    target: DropTarget,
    point: Point,
}

#[derive(Clone, Copy)]
struct NativeMoveSession {
    group: FloatingGroupId,
    native_window: NativeWindowId,
    kind: NativeMoveKind,
    grab_offset: PhysicalPoint,
    initial_cursor: PhysicalPoint,
    start_bounds: Rect,
    start_pointer: Point,
    drag_threshold: f32,
    threshold_crossed: bool,
    high_resolution_timer: bool,
    drop_target: Option<NativeMoveTarget>,
}

#[derive(Clone, Copy)]
struct FloatingGroupSnapshot {
    id: FloatingGroupId,
    bounds: Rect,
    z_order: i32,
    minimum_size: Option<Size>,
}

#[derive(Clone, Copy)]
struct NativeResizeSession {
    group: FloatingGroupId,
    window_id: WindowId,
    transaction_started: bool,
}

/// Reconciles the native-window registry with the current Core Layout.
pub struct FloatingWindowManager {
    controller: Entity<DockController>,
    theme: DockTheme,
    main_native_window: Option<NativeWindowId>,
    main_host_origin: Point,
    native_move: Option<NativeMoveSession>,
    native_resize: Option<NativeResizeSession>,
    reconciling: bool,
    registry: WindowRegistry,
    native_ops: SystemNativeWindowOps,
    controller_subscription: Option<Subscription>,
}

impl FloatingWindowManager {
    pub fn new(controller: Entity<DockController>, theme: DockTheme) -> Self {
        Self {
            controller,
            theme,
            main_native_window: None,
            main_host_origin: Point::ZERO,
            native_move: None,
            native_resize: None,
            reconciling: false,
            registry: WindowRegistry::default(),
            native_ops: SystemNativeWindowOps,
            controller_subscription: None,
        }
    }

    pub fn registry(&self) -> &WindowRegistry {
        &self.registry
    }

    /// Connects the manager to the main GPUI window and the shared controller.
    ///
    /// The main HWND is retained as the native owner for every floating window.
    /// The first reconciliation also restores windows for groups already present
    /// in a workspace loaded before DockHost was rendered.
    pub(crate) fn attach_main_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.main_native_window.is_none() {
            match self.native_ops.hwnd_of(window) {
                Ok(native_window) => self.main_native_window = Some(native_window),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "main window native handle is unavailable; floating windows will use GPUI visibility"
                    );
                }
            }
        }

        if self.controller_subscription.is_none() {
            let controller = self.controller.clone();
            self.controller_subscription = Some(cx.subscribe(
                &controller,
                |manager, _, event: &DockEvent, manager_cx| {
                    if matches!(event, DockEvent::LayoutChanged { .. }) {
                        manager.reconcile(manager_cx);
                    }
                },
            ));
        }

        self.reconcile(cx);
    }

    /// Records the main DockHost origin inside the main window's client area.
    pub(crate) fn set_main_host_origin(&mut self, origin: Point) {
        if origin.validate().is_err() || self.main_host_origin == origin {
            return;
        }
        // FloatingGroup bounds already store virtual-desktop physical pixels.
        // This origin is used only to translate main-host input coordinates.
        self.main_host_origin = origin;
    }

    /// Converts a proposed main-host-local logical rectangle to the Core
    /// floating-bounds contract: physical virtual-desktop origin + logical size.
    pub(crate) fn screen_bounds_from_main_local(&self, bounds: Rect) -> Option<Rect> {
        bounds.validate_non_empty().ok()?;
        let main = self.main_native_window?;
        let client = self.native_ops.client_origin(main).ok()?;
        let scale = self.native_ops.scale_factor(main).ok()?;
        let origin = screen_position_from_main_local(
            bounds.origin,
            self.main_host_origin,
            client,
            scale,
        )?;
        Some(Rect::new(Point::new(origin.x as f32, origin.y as f32), bounds.size))
    }

    /// Converts a rectangle in one floating host's local logical coordinates
    /// to the Core contract: physical virtual-desktop origin + logical size.
    pub(crate) fn screen_bounds_from_floating_local(
        &self,
        source_group: FloatingGroupId,
        bounds: Rect,
    ) -> Option<Rect> {
        bounds.validate_non_empty().ok()?;
        let native_window = self.registry.floating.get(&source_group)?.native_window?;
        let client = self.native_ops.client_origin(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        let origin = screen_position_from_main_local(
            bounds.origin,
            Point::ZERO,
            client,
            scale,
        )?;
        Some(Rect::new(Point::new(origin.x as f32, origin.y as f32), bounds.size))
    }

    /// Returns true when the physical cursor is over another registered
    /// docking window, even if the source window still owns pointer capture.
    pub(crate) fn cursor_over_other_docking_window(
        &self,
        source_group: FloatingGroupId,
    ) -> bool {
        let Some(source_window) = self
            .registry
            .floating
            .get(&source_group)
            .and_then(|entry| entry.native_window)
        else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };

        let mut known = Vec::new();
        if let Some(main) = self.main_native_window {
            known.push(main);
        }
        known.extend(
            self.registry
                .floating
                .values()
                .filter_map(|entry| entry.native_window),
        );

        self.native_ops
            .window_from_point(cursor, &known, None)
            .ok()
            .flatten()
            .is_some_and(|target_window| target_window != source_window)
    }

    pub(crate) fn native_move_active(&self) -> bool { self.native_move.is_some() }

    /// Starts polling a native OS sizing loop after its first GPUI redraw.
    pub(crate) fn observe_native_resize(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        if self.native_move.is_some() || self.native_resize.is_some() {
            return;
        }
        let Some(entry) = self.registry.floating.get(&group) else {
            return;
        };
        if entry.window_id != window_id {
            return;
        }
        let Some(native_window) = entry.native_window else {
            return;
        };
        if !self.native_ops.is_resizing(native_window).unwrap_or(false) {
            return;
        }

        self.native_resize = Some(NativeResizeSession {
            group,
            window_id,
            transaction_started: false,
        });
        self.schedule_native_resize(cx, group, window_id);
    }

    fn schedule_native_resize(
        &self,
        cx: &mut Context<Self>,
        group: FloatingGroupId,
        window_id: WindowId,
    ) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor().timer(Duration::from_millis(8)).await;
                let Ok(keep_polling) = manager.update(app, |manager, manager_cx| {
                    manager.tick_native_resize(group, window_id, manager_cx)
                }) else {
                    break;
                };
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_native_resize(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut state) = self.native_resize.filter(|state| {
            state.group == group && state.window_id == window_id
        }) else {
            return false;
        };

        let native_window = self.registry.floating.get(&group).and_then(|entry| {
            (entry.window_id == window_id).then_some(entry.native_window).flatten()
        });
        let Some(native_window) = native_window else {
            self.finish_native_resize_transaction(state, cx);
            return false;
        };

        let resizing = self.native_ops.is_resizing(native_window).unwrap_or(false);
        if let Some(bounds) = self.core_bounds_from_native_window(native_window) {
            let current = self.controller.update(cx, |controller, _| {
                controller.layout().floating_group(group).map(|floating| floating.bounds)
            });
            if let Some(current) = current
                && current != bounds
            {
                if !state.transaction_started {
                    state.transaction_started = self.controller.update(cx, |controller, _| {
                        controller.begin_native_resize_transaction()
                    });
                }
                if state.transaction_started {
                    let result = self.controller.update(cx, |controller, controller_cx| {
                        controller.dispatch_in(
                            DockCommand::ResizeFloatingBounds { group, bounds },
                            controller_cx,
                        )
                    });
                    if let Err(error) = result {
                        tracing::warn!(
                            ?group,
                            %error,
                            "could not synchronize native floating-window bounds with Core"
                        );
                    }
                }
            }
        }

        if !resizing {
            self.finish_native_resize_transaction(state, cx);
            return false;
        }

        self.native_resize = Some(state);
        true
    }

    fn finish_native_resize_transaction(
        &mut self,
        state: NativeResizeSession,
        cx: &mut Context<Self>,
    ) {
        self.native_resize = None;
        if state.transaction_started {
            self.controller.update(cx, |controller, controller_cx| {
                controller.finish_native_resize_transaction();
                controller_cx.notify();
            });
        }

        // A resize can finish on a monitor with a different DPI. Re-read the
        // final window scale after the modal loop has exited and resize the
        // native frame to the preserved Core logical size at that scale.
        self.reconcile(cx);
    }

    fn core_bounds_from_native_window(&self, native_window: NativeWindowId) -> Option<Rect> {
        let rect = self.native_ops.window_rect(native_window).ok()?;
        let physical_width = rect.right.checked_sub(rect.left)?;
        let physical_height = rect.bottom.checked_sub(rect.top)?;
        if physical_width <= 0 || physical_height <= 0 {
            return None;
        }
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }

        let bounds = Rect::new(
            Point::new(rect.left as f32, rect.top as f32),
            Size::new(
                physical_width as f32 / scale,
                physical_height as f32 / scale,
            ),
        );
        bounds.validate_non_empty().ok()?;
        Some(bounds)
    }

    /// Starts native movement for a group created by the current main-window drag.
    pub(crate) fn begin_tearoff_move(
        &mut self,
        group: FloatingGroupId,
        pointer: Point,
        local_bounds: Rect,
        model_bounds: Rect,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() { return false; }
        let Some(entry) = self.registry.floating.get(&group) else { return false; };
        let (Some(native_window), window_id) = (entry.native_window, entry.window_id) else { return false; };
        let Ok(cursor) = self.native_ops.cursor_position() else { return false; };
        let scale = self.main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or(1.0);
        let Some(offset_x) = physical_coordinate(pointer.x - local_bounds.origin.x, scale) else { return false; };
        let Some(offset_y) = physical_coordinate(pointer.y - local_bounds.origin.y, scale) else { return false; };
        let grab_offset = PhysicalPoint { x: offset_x, y: offset_y };
        let position = PhysicalPoint {
            x: cursor.x.saturating_sub(grab_offset.x),
            y: cursor.y.saturating_sub(grab_offset.y),
        };
        if let Err(error) = self.native_ops.move_window(native_window, position) {
            tracing::warn!(?group, %error, "could not start native tear-off movement");
            return false;
        }
        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group, native_window, kind: NativeMoveKind::TearOff, grab_offset, initial_cursor: cursor,
            start_bounds: model_bounds, start_pointer: pointer, drag_threshold: 0.0, threshold_crossed: true,
            high_resolution_timer, drop_target: None,
        });
        self.schedule_native_move(cx, group, window_id);
        true
    }

    /// Starts native movement for a newly-created floating window whose
    /// source was detached from a tab inside another floating host.
    pub(crate) fn begin_floating_tab_tearoff_move(
        &mut self,
        group: FloatingGroupId,
        model_bounds: Rect,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() {
            return false;
        }
        let Some(entry) = self.registry.floating.get(&group) else {
            return false;
        };
        let (Some(native_window), window_id) = (entry.native_window, entry.window_id) else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let Some(origin_x) = integer_physical_coordinate(model_bounds.origin.x) else {
            return false;
        };
        let Some(origin_y) = integer_physical_coordinate(model_bounds.origin.y) else {
            return false;
        };
        let grab_offset = PhysicalPoint {
            x: cursor.x.saturating_sub(origin_x),
            y: cursor.y.saturating_sub(origin_y),
        };
        let position = PhysicalPoint {
            x: cursor.x.saturating_sub(grab_offset.x),
            y: cursor.y.saturating_sub(grab_offset.y),
        };
        if let Err(error) = self.native_ops.move_window(native_window, position) {
            tracing::warn!(?group, %error, "could not start native floating-tab tear-off movement");
            return false;
        }

        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group,
            native_window,
            kind: NativeMoveKind::TearOff,
            grab_offset,
            initial_cursor: cursor,
            start_bounds: model_bounds,
            start_pointer: Point::ZERO,
            drag_threshold: 0.0,
            threshold_crossed: true,
            high_resolution_timer,
            drop_target: None,
        });
        self.schedule_native_move(cx, group, window_id);
        true
    }

    /// Starts a thresholded native move from an already-floating title grip.
    pub(crate) fn begin_existing_window_move(
        &mut self, group: FloatingGroupId, pointer: Point, cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() { return false; }
        let Some(entry) = self.registry.floating.get(&group) else { return false; };
        let (Some(native_window), window_id) = (entry.native_window, entry.window_id) else { return false; };
        let Ok(cursor) = self.native_ops.cursor_position() else { return false; };
        let Ok(native_rect) = self.native_ops.window_rect(native_window) else { return false; };
        let start_bounds = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        });
        let Some(start_bounds) = start_bounds else { return false; };
        let metrics = self.controller.update(cx, |controller, _| controller.metrics());
        if !self.controller.update(cx, |controller, _| controller.begin_floating_window_move(group, pointer)) {
            return false;
        }
        let scale = self.native_ops.scale_factor(native_window).ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or(1.0);
        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group, native_window, kind: NativeMoveKind::ExistingWindow,
            grab_offset: PhysicalPoint {
                x: cursor.x.saturating_sub(native_rect.left),
                y: cursor.y.saturating_sub(native_rect.top),
            },
            initial_cursor: cursor, start_bounds, start_pointer: pointer,
            drag_threshold: metrics.drag_threshold * scale, threshold_crossed: false,
            high_resolution_timer, drop_target: None,
        });
        self.schedule_native_move(cx, group, window_id);
        true
    }

    fn schedule_native_move(&self, cx: &mut Context<Self>, group: FloatingGroupId, window_id: WindowId) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor().timer(Duration::from_millis(4)).await;
                let Ok(keep_moving) = manager.update(app, |manager, manager_cx| {
                    manager.tick_native_move(group, window_id, manager_cx)
                }) else {
                    break;
                };
                if !keep_moving {
                    break;
                }
            }
        }).detach();
    }

    fn tick_native_move(&mut self, group: FloatingGroupId, window_id: WindowId, cx: &mut Context<Self>) -> bool {
        let Some(mut state) = self.native_move.filter(|state| state.group == group) else { return false; };
        if self.registry.floating.get(&group).is_none_or(|entry| entry.window_id != window_id) {
            self.native_move = None;
            if state.high_resolution_timer {
                let _ = self.native_ops.end_high_resolution_timer();
            }
            self.cancel_controller_move(cx);
            return false;
        }
        let pointer_interaction_active = self.controller.update(cx, |controller, _| {
            controller.has_active_pointer_interaction()
        });
        if !pointer_interaction_active {
            self.cancel_native_move(state, cx);
            return false;
        }
        if self.native_ops.key_down(NativeKey::Escape).unwrap_or(false) {
            self.cancel_native_move(state, cx);
            return false;
        }
        let Ok(cursor) = self.native_ops.cursor_position() else {
            self.cancel_native_move(state, cx);
            return false;
        };
        if !state.threshold_crossed {
            let dx = cursor.x.saturating_sub(state.initial_cursor.x);
            let dy = cursor.y.saturating_sub(state.initial_cursor.y);
            let distance = ((dx as f64).powi(2) + (dy as f64).powi(2)).sqrt();
            if distance < state.drag_threshold as f64 {
                if !self.native_ops.left_button_down().unwrap_or(false) {
                    self.finish_native_move(state, cx);
                    return false;
                }
                return true;
            }
            state.threshold_crossed = true;
            let scale = self.native_ops.scale_factor(state.native_window).ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or(1.0);
            let point = Point::new(state.start_pointer.x + dx as f32 / scale, state.start_pointer.y + dy as f32 / scale);
            self.controller.update(cx, |controller, _| {
                if matches!(controller.drag().session().state(), DragState::Pressed { .. }) {
                    let _ = controller.drag_mut().session_mut().pointer_move(point);
                }
            });
        }
        let position = PhysicalPoint {
            x: cursor.x.saturating_sub(state.grab_offset.x),
            y: cursor.y.saturating_sub(state.grab_offset.y),
        };
        if let Err(error) = self.native_ops.move_window(state.native_window, position) {
            tracing::warn!(?group, %error, "native floating-window move failed");
            self.cancel_native_move(state, cx);
            return false;
        }
        self.update_native_drop_target(&mut state, cursor, cx);
        self.native_move = Some(state);
        if !self.native_ops.left_button_down().unwrap_or(false) {
            self.finish_native_move(state, cx);
            return false;
        }
        true
    }

    fn update_native_drop_target(&self, state: &mut NativeMoveSession, cursor: PhysicalPoint, cx: &mut Context<Self>) {
        let source = self.controller.update(cx, |controller, _| {
            let payload = match controller.drag().session().state() {
                DragState::Dragging { payload, .. } => payload,
                _ => return None,
            };
            match payload {
                DragPayload::Panel { panel } => Some(DropSource::Panel { panel }),
                DragPayload::Group { group } => Some(DropSource::Group { group }),
                DragPayload::FloatingGroup { group } => controller
                    .layout()
                    .floating_group(group)
                    .and_then(|floating| controller.layout().dock_area(floating.area))
                    .map(|area| DropSource::Group { group: area.root }),
            }
        });
        let Some(source) = source else {
            state.drop_target = None;
            self.clear_native_drag_target(cx);
            return;
        };
        let mut known = Vec::new();
        if let Some(main) = self.main_native_window { known.push(main); }
        known.extend(self.registry.floating.values().filter_map(|entry| entry.native_window));
        let target_window = self.native_ops.window_from_point(cursor, &known, Some(state.native_window)).ok().flatten();
        let Some(target_window) = target_window else {
            state.drop_target = None;
            self.clear_native_drag_target(cx);
            return;
        };
        let resolved = if Some(target_window) == self.main_native_window {
            let Some(point) = self.core_point_for_main(cursor) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            self.controller.update(cx, |controller, controller_cx| {
                let target = match controller.prepare_drop_resolver(source)
                    .and_then(|resolver| resolver.resolve(point))
                {
                    Ok(target) => target,
                    Err(_) => {
                        controller.drag_mut().clear_resolver();
                        let _ = controller.drag_mut().session_mut().pointer_move(point);
                        let _ = controller.drag_mut().session_mut().set_drop_target(None);
                        return None;
                    }
                };
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                controller_cx.notify();
                target.map(|target| NativeMoveTarget { target, point })
            })
        } else {
            let Some((target_group, target_id)) = self.registry.floating.iter().find_map(|(group, entry)| {
                (entry.native_window == Some(target_window) && *group != state.group).then_some((*group, entry.window_id))
            }) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            let Some(point) = self.core_point_for_floating(target_window, cursor) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            self.controller.update(cx, |controller, controller_cx| {
                let target = match controller
                    .prepare_floating_window_drop_resolver(target_id, target_group, source)
                    .and_then(|resolver| resolver.resolve(point))
                {
                    Ok(target) => target,
                    Err(_) => {
                        controller.drag_mut().clear_resolver();
                        let _ = controller.drag_mut().session_mut().pointer_move(point);
                        let _ = controller.drag_mut().session_mut().set_drop_target(None);
                        return None;
                    }
                };
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                controller_cx.notify();
                target.map(|target| NativeMoveTarget { target, point })
            })
        };
        state.drop_target = resolved;
    }


    fn clear_native_drag_target(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            controller_cx.notify();
        });
    }

    fn core_point_for_main(&self, screen: PhysicalPoint) -> Option<Point> {
        let main = self.main_native_window?;
        let client = self.native_ops.client_origin(main).ok()?;
        let scale = self.native_ops.scale_factor(main).ok()?;
        if !scale.is_finite() || scale <= 0.0 { return None; }
        main_local_point_from_screen(screen, client, self.main_host_origin, scale)
    }

    fn core_point_for_floating(&self, window: NativeWindowId, screen: PhysicalPoint) -> Option<Point> {
        let client = self.native_ops.client_origin(window).ok()?;
        let scale = self.native_ops.scale_factor(window).ok()?;
        if !scale.is_finite() || scale <= 0.0 { return None; }
        let point = Point::new((screen.x - client.x) as f32 / scale, (screen.y - client.y) as f32 / scale);
        point.validate().ok()?;
        Some(point)
    }

    fn finish_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        self.native_move = None;
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }
        let origin = self.native_ops.window_rect(state.native_window).ok()
            .and_then(|rect| self.core_origin_from_screen(PhysicalPoint { x: rect.left, y: rect.top }))
            .unwrap_or(state.start_bounds.origin);
        // The target may belong to the main window or another floating window.
        // Both have already resolved their own local coordinates into a Core target.
        let drop_target = state.drop_target.map(|target| (target.target, target.point));
        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_tearoff_move_in(state.group, origin, drop_target, controller_cx)
        });
        if let Err(error) = result {
            tracing::warn!(?state.group, %error, "could not commit native floating-window move");
            self.cancel_controller_move(cx);
        }
    }

    fn cancel_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        self.native_move = None;
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }
        let _ = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        });
        if state.kind == NativeMoveKind::ExistingWindow
            && let Some(position) = self.physical_origin_for_bounds(state.start_bounds)
            && let Err(error) = self.native_ops.move_window(state.native_window, position)
        {
            tracing::warn!(?state.group, %error, "could not restore floating window after Escape");
        }
    }

    fn cancel_controller_move(&mut self, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        });
    }

    fn physical_origin_for_bounds(&self, bounds: Rect) -> Option<PhysicalPoint> {
        Some(PhysicalPoint {
            x: integer_physical_coordinate(bounds.origin.x)?,
            y: integer_physical_coordinate(bounds.origin.y)?,
        })
    }

    fn core_origin_from_screen(&self, screen: PhysicalPoint) -> Option<Point> {
        let point = Point::new(screen.x as f32, screen.y as f32);
        point.validate().ok()?;
        Some(point)
    }

    /// WindowOptions bounds use logical screen coordinates; Core stores physical
    /// screen origins. The main monitor scale is the bootstrap approximation until
    /// monitor-specific DPI selection is handled by the multi-monitor stage.
    fn gpui_origin_for_bounds(&self, bounds: Rect) -> Point {
        let scale = self.main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        Point::new(bounds.origin.x / scale, bounds.origin.y / scale)
    }

    /// Makes the registry match the current floating groups in Core.
    ///
    /// This method intentionally snapshots only group metadata and panel
    /// ownership. It never clones Layout or builds a second docking tree.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconciling {
            return;
        }
        self.reconciling = true;

        let (mut groups, panel_owners, restore_pending) = self.controller.update(cx, |controller, _| {
            let restore_pending = controller.take_workspace_restore_pending();
            let layout = controller.layout();
            let metrics = controller.metrics();
            let mut panel_owners = HashMap::new();

            for side in [DockSide::Left, DockSide::Right] {
                if let Some(area) = layout.main_area(side) {
                    let mut panels = HashSet::new();
                    collect_panel_ids(layout, area.root, &mut panels);
                    for panel in panels {
                        panel_owners.insert(panel, PanelWindowOwner::Main);
                    }
                }
            }

            let mut groups = Vec::new();
            for (id, group) in layout.floating_groups() {
                groups.push(FloatingGroupSnapshot {
                    id,
                    bounds: group.bounds,
                    z_order: group.z_order,
                    minimum_size: layout.floating_minimum_size(id, &metrics).ok(),
                });

                if let Some(area) = layout.dock_area(group.area) {
                    let mut panels = HashSet::new();
                    collect_panel_ids(layout, area.root, &mut panels);
                    for panel in panels {
                        panel_owners.insert(panel, PanelWindowOwner::Floating(id));
                    }
                }
            }

            (groups, panel_owners, restore_pending)
        });

        let monitor_work_areas = if restore_pending {
            match self.native_ops.monitor_work_areas() {
                Ok(areas) => Some(areas),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "could not enumerate monitor work areas for workspace recovery"
                    );
                    None
                }
            }
        } else {
            None
        };

        groups.sort_by_key(|group| group.z_order);
        let desired_groups = groups
            .iter()
            .map(|group| group.id)
            .collect::<HashSet<_>>();

        // A Core group can disappear when it is merged into another group or
        // explicitly closed. Hide its native window and keep it available for
        // a later extraction instead of destroying and recreating the HWND.
        let removed_groups = self
            .registry
            .floating
            .keys()
            .filter(|group| !desired_groups.contains(group))
            .copied()
            .collect::<Vec<_>>();
        for group in removed_groups {
            self.release_group_window(group, cx);
        }

        let mut monitor_corrections = Vec::new();
        for group in groups.iter().copied() {
            let mut entry = self.registry.floating.remove(&group.id);

            if let Some(existing) = entry.as_ref()
                && existing.window.update(cx, |_, _, _| ()).is_err()
            {
                // A window may have been closed by the platform. Do not keep a
                // stale handle in the registry; create a replacement below.
                entry = None;
            }

            let mut entry = match entry {
                Some(entry) => Some(entry),
                None => self.take_reusable_window(group, cx),
            };

            if entry.is_none() {
                entry = self.create_window(group, cx);
            }

            let Some(mut entry) = entry else {
                tracing::error!(
                    ?group.id,
                    "could not create or reuse a floating native window"
                );
                continue;
            };

            self.sync_window_bounds(&mut entry, group);
            if let Some(areas) = monitor_work_areas.as_deref()
                && let Some(corrected_bounds) =
                    self.restore_native_window_to_monitor(&mut entry, areas)
            {
                monitor_corrections.push((group.id, corrected_bounds));
                entry.bounds = corrected_bounds;
            }
            self.sync_native_resize(&mut entry, group);
            entry.z_order = group.z_order;
            self.registry.floating.insert(group.id, entry);
        }

        self.registry.panel_owners = panel_owners;

        // Re-showing without activation in ascending z-order restores the
        // stacking order without stealing focus from the main application.
        for group in groups {
            if let Some(native_window) = self
                .registry
                .floating
                .get(&group.id)
                .and_then(|entry| entry.native_window)
            {
                if let Err(error) = self.native_ops.show_no_activate(native_window) {
                    tracing::warn!(
                        ?group.id,
                        %error,
                        "could not update floating-window visibility or z-order"
                    );
                }
            }
        }

        if !monitor_corrections.is_empty() {
            let result = self.controller.update(cx, |controller, controller_cx| {
                let result = controller.apply_workspace_monitor_bounds(&monitor_corrections);
                if let Err(error) = result {
                    tracing::warn!(
                        %error,
                        "could not persist monitor recovery in the restored Core layout"
                    );
                }
                controller.flush_events(controller_cx);
                controller_cx.notify();
            });
            if let Err(error) = result {
                tracing::warn!(
                    %error,
                    "could not update the controller after monitor recovery"
                );
            }
        }

        self.reconciling = false;
    }

    fn release_group_window(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) {
        let Some(mut entry) = self.registry.floating.remove(&group) else {
            return;
        };

        if let Some(native_window) = entry.native_window {
            if entry.resize_enabled {
                if let Err(error) = self.native_ops.disable_resize(native_window) {
                    tracing::warn!(
                        ?group,
                        %error,
                        "could not disable native resize on a released floating window"
                    );
                }
                entry.resize_enabled = false;
            }
            if entry.native_min_size.is_some() {
                if let Err(error) = self.native_ops.set_min_size(native_window, None) {
                    tracing::warn!(
                        ?group,
                        %error,
                        "could not clear the native minimum size on a released floating window"
                    );
                }
                entry.native_min_size = None;
            }
            if let Err(error) = self.native_ops.hide(native_window) {
                tracing::warn!(
                    ?group,
                    %error,
                    "could not hide a released floating window"
                );
            }
            self.registry.reusable.push(entry);
        } else {
            // Platforms without native hide support cannot safely retain a
            // hidden GPUI window. Close it and create a fresh one if needed.
            let _ = entry
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    }

    fn take_reusable_window(
        &mut self,
        group: FloatingGroupSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<FloatingWindowEntry> {
        while let Some(mut entry) = self.registry.reusable.pop() {
            let result = entry.window.update(cx, |host, _, host_cx| {
                host.set_group(group.id);
                host_cx.notify();
            });
            if let Err(error) = result {
                tracing::debug!(
                    ?group.id,
                    %error,
                    "discarding an unavailable reusable floating window"
                );
                continue;
            }

            self.sync_window_bounds(&mut entry, group);
            self.sync_native_resize(&mut entry, group);
            entry.z_order = group.z_order;
            return Some(entry);
        }

        None
    }

    fn create_window(
        &mut self,
        group: FloatingGroupSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<FloatingWindowEntry> {
        let controller = self.controller.clone();
        let theme = self.theme;
        let manager = cx.entity().downgrade();
        let show_immediately = self.main_native_window.is_none();
        let logical_origin = self.gpui_origin_for_bounds(group.bounds);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(logical_origin.x), px(logical_origin.y)),
                size(px(group.bounds.size.width), px(group.bounds.size.height)),
            ))),
            titlebar: None,
            focus: false,
            show: show_immediately,
            kind: WindowKind::PopUp,
            is_movable: true,
            app_owns_titlebar_drag: true,
            is_resizable: true,
            is_minimizable: false,
            ..Default::default()
        };

        let window = match cx.open_window(options, move |_, app| {
            app.new(|_| FloatingHost::new(controller, group.id, theme, manager))
        }) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(
                    ?group.id,
                    %error,
                    "GPUI could not open a floating window"
                );
                return None;
            }
        };

        let window_id = window
            .update(cx, |_, window, _| window.window_handle().window_id())
            .ok()?;
        let native_ops = self.native_ops;
        let native_window = match window.update(cx, move |_, window, _| {
            native_ops.hwnd_of(window)
        }) {
            Ok(Ok(native_window)) => Some(native_window),
            Ok(Err(error)) => {
                tracing::debug!(
                    ?group.id,
                    %error,
                    "floating window has no native handle"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    ?group.id,
                    %error,
                    "could not inspect a newly created floating window"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };

        if self.main_native_window.is_some() && native_window.is_none() {
            // On Windows a hidden window without an HWND cannot be shown or
            // assigned an owner. Fail closed rather than leave an invisible
            // GPUI window registered as if it were usable.
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        }

        if let Some(native_window) = native_window {
            if let Some(owner) = self.main_native_window
                && let Err(error) = self.native_ops.set_owner(native_window, owner)
            {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not assign the main window as floating-window owner"
                );
            }
            if let Some(position) = self.physical_origin_for_bounds(group.bounds)
                && let Err(error) = self.native_ops.move_window(native_window, position)
            {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not place a floating window relative to the main workspace"
                );
            }
            if let Err(error) = self.native_ops.remove_topmost(native_window) {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not clear the floating window topmost state"
                );
            }
        }

        let mut entry = FloatingWindowEntry {
            window,
            window_id,
            native_window,
            bounds: group.bounds,
            z_order: group.z_order,
            native_min_size: None,
            native_scale_factor: None,
            resize_enabled: false,
        };
        self.sync_native_resize(&mut entry, group);
        Some(entry)
    }

    /// Repositions a restored window only when its title area is no longer
    /// reachable on any connected monitor. Returns corrected Core bounds.
    fn restore_native_window_to_monitor(
        &self,
        entry: &mut FloatingWindowEntry,
        work_areas: &[platform::PhysicalRect],
    ) -> Option<Rect> {
        let native_window = entry.native_window?;
        if work_areas.is_empty() {
            return None;
        }
        let current = self.native_ops.window_rect(native_window).ok()?;
        if native_rect_has_reachable_title_area(current, work_areas) {
            return None;
        }
        let work_area = self.preferred_monitor_work_area(work_areas)?;
        let position = restored_window_origin(current, work_area)?;
        if position.x == current.left && position.y == current.top {
            return None;
        }
        if let Err(error) = self.native_ops.move_window(native_window, position) {
            tracing::warn!(
                %error,
                "could not move an off-screen restored floating window onto a connected monitor"
            );
            return None;
        }

        let destination_scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        if let (Some(width), Some(height)) = (
            physical_coordinate(entry.bounds.size.width, destination_scale),
            physical_coordinate(entry.bounds.size.height, destination_scale),
        ) {
            let desired_size = PhysicalSize { width, height };
            if desired_size.is_valid()
                && let Err(error) = self.native_ops.resize_window(native_window, desired_size)
            {
                tracing::warn!(
                    %error,
                    "could not rescale a restored floating window for its destination monitor"
                );
            }
        }

        let corrected = self.core_bounds_from_native_window(native_window)?;
        entry.native_scale_factor = Some(destination_scale);
        Some(corrected)
    }

    fn preferred_monitor_work_area(
        &self,
        work_areas: &[platform::PhysicalRect],
    ) -> Option<platform::PhysicalRect> {
        if let Some(main) = self.main_native_window
            && let Ok(rect) = self.native_ops.window_rect(main)
        {
            let center = PhysicalPoint {
                x: (rect.left as i64 + rect.right as i64).div_euclid(2) as i32,
                y: (rect.top as i64 + rect.bottom as i64).div_euclid(2) as i32,
            };
            if let Some(area) = work_areas.iter().copied().find(|area| area.contains(center)) {
                return Some(area);
            }
        }
        work_areas.first().copied()
    }

    fn sync_native_resize(
        &self,
        entry: &mut FloatingWindowEntry,
        group: FloatingGroupSnapshot,
    ) {
        let Some(native_window) = entry.native_window else {
            return;
        };

        if !entry.resize_enabled {
            match self.native_ops.enable_resize(native_window) {
                Ok(()) => entry.resize_enabled = true,
                Err(error) => tracing::warn!(
                    ?group.id,
                    %error,
                    "could not enable native floating-window resize"
                ),
            }
        }

        let desired_minimum = group.minimum_size.and_then(|minimum| {
            let scale = self.native_ops.scale_factor(native_window)
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .unwrap_or(1.0);
            let width = physical_coordinate(minimum.width, scale)?;
            let height = physical_coordinate(minimum.height, scale)?;
            let size = PhysicalSize { width, height };
            size.is_valid().then_some(size)
        });

        if entry.native_min_size != desired_minimum {
            match self.native_ops.set_min_size(native_window, desired_minimum) {
                Ok(()) => entry.native_min_size = desired_minimum,
                Err(error) => tracing::warn!(
                    ?group.id,
                    %error,
                    "could not synchronize native floating-window minimum size"
                ),
            }
        }
    }

    fn sync_window_bounds(
        &self,
        entry: &mut FloatingWindowEntry,
        group: FloatingGroupSnapshot,
    ) {
        let scale = entry.native_window.and_then(|native_window| {
            self.native_ops.scale_factor(native_window).ok()
        })
        .filter(|scale| scale.is_finite() && *scale > 0.0);

        let scale_changed = match (entry.native_scale_factor, scale) {
            (Some(previous), Some(current)) => (previous - current).abs() > 0.001,
            (None, Some(_)) => true,
            _ => false,
        };
        if entry.bounds == group.bounds && !scale_changed {
            return;
        }

        // During an OS-owned modal resize the native frame is authoritative.
        // Core has just received that frame's latest bounds; sending SetWindowPos
        // back to the same HWND here can fight the active sizing loop.
        if self
            .native_resize
            .is_some_and(|session| session.group == group.id)
        {
            entry.bounds = group.bounds;
            entry.native_scale_factor = scale;
            return;
        }

        if let Some(native_window) = entry.native_window {
            if entry.bounds.origin != group.bounds.origin
                && let Some(position) = self.physical_origin_for_bounds(group.bounds)
                && let Err(error) = self.native_ops.move_window(native_window, position)
            {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not move a floating native window"
                );
            }

            // Moving to another monitor can synchronously update the HWND DPI.
            // Query again after SetWindowPos rather than sizing with the source
            // monitor's cached scale.
            let effective_scale = self
                .native_ops
                .scale_factor(native_window)
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .or(scale)
                .unwrap_or(1.0);
            let effective_scale_changed = entry.native_scale_factor.is_none_or(|previous| {
                (previous - effective_scale).abs() > 0.001
            });

            if entry.bounds.size != group.bounds.size || effective_scale_changed {
                if let (Some(width), Some(height)) = (
                    physical_coordinate(group.bounds.size.width, effective_scale),
                    physical_coordinate(group.bounds.size.height, effective_scale),
                ) {
                    let size = PhysicalSize { width, height };
                    if size.is_valid()
                        && let Err(error) = self.native_ops.resize_window(native_window, size)
                    {
                        tracing::warn!(
                            ?group.id,
                            %error,
                            "could not resize a floating native window"
                        );
                    }
                }
            }

            entry.native_scale_factor = Some(effective_scale);
        } else {
            entry.native_scale_factor = None;
        }

        entry.bounds = group.bounds;
    }
}


fn native_rect_has_reachable_title_area(
    rect: platform::PhysicalRect,
    work_areas: &[platform::PhysicalRect],
) -> bool {
    let width = rect.right.saturating_sub(rect.left).max(0);
    let height = rect.bottom.saturating_sub(rect.top).max(0);
    if width == 0 || height == 0 {
        return false;
    }

    work_areas.iter().any(|area| {
        let required_width = width.min(80).min(area.width().max(0));
        let required_height = height.min(40).min(area.height().max(0));
        let visible_width = rect.right.min(area.right).saturating_sub(rect.left.max(area.left));
        let visible_height = rect.top.saturating_add(required_height)
            .min(area.bottom)
            .saturating_sub(rect.top.max(area.top));
        visible_width >= required_width && visible_height >= required_height
    })
}

fn restored_window_origin(
    rect: platform::PhysicalRect,
    work_area: platform::PhysicalRect,
) -> Option<PhysicalPoint> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    let area_width = work_area.right.checked_sub(work_area.left)?;
    let area_height = work_area.bottom.checked_sub(work_area.top)?;
    if width <= 0 || height <= 0 || area_width <= 0 || area_height <= 0 {
        return None;
    }

    // Keep the left side of the title strip and its top edge reachable. A
    // window larger than the work area may extend beyond it, but stays usable.
    let visible_width = width.min(80).min(area_width);
    let visible_height = height.min(40).min(area_height);
    let minimum_x = work_area.left as i64;
    let maximum_x = work_area.right as i64 - visible_width as i64;
    let minimum_y = work_area.top as i64;
    let maximum_y = work_area.bottom as i64 - visible_height as i64;
    let x = (rect.left as i64).clamp(minimum_x, maximum_x);
    let y = (rect.top as i64).clamp(minimum_y, maximum_y);

    Some(PhysicalPoint {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
    })
}

fn screen_position_from_main_local(
    local: Point,
    host_origin: Point,
    client_origin: PhysicalPoint,
    scale: f32,
) -> Option<PhysicalPoint> {
    if !scale.is_finite() || scale <= 0.0 { return None; }
    let x = physical_coordinate(host_origin.x + local.x, scale)?;
    let y = physical_coordinate(host_origin.y + local.y, scale)?;
    Some(PhysicalPoint {
        x: client_origin.x.checked_add(x)?,
        y: client_origin.y.checked_add(y)?,
    })
}

fn main_local_point_from_screen(
    screen: PhysicalPoint,
    client_origin: PhysicalPoint,
    host_origin: Point,
    scale: f32,
) -> Option<Point> {
    if !scale.is_finite() || scale <= 0.0 { return None; }
    let point = Point::new(
        (screen.x - client_origin.x) as f32 / scale - host_origin.x,
        (screen.y - client_origin.y) as f32 / scale - host_origin.y,
    );
    point.validate().ok()?;
    Some(point)
}

fn integer_physical_coordinate(value: f32) -> Option<i32> {
    (value.is_finite() && value >= i32::MIN as f32 && value <= i32::MAX as f32)
        .then(|| value.round() as i32)
}

fn physical_coordinate(value: f32, scale: f32) -> Option<i32> {
    let scaled = value * scale;
    (scaled.is_finite() && scaled >= i32::MIN as f32 && scaled <= i32::MAX as f32)
        .then(|| scaled.round() as i32)
}

#[cfg(test)]
mod coordinate_tests {
    use super::{
        main_local_point_from_screen, native_rect_has_reachable_title_area,
        restored_window_origin, screen_position_from_main_local,
    };
    use dock_core::Point;
    use platform::{PhysicalPoint, PhysicalRect};

    #[test]
    fn off_screen_restored_window_is_placed_inside_the_preferred_work_area() {
        let window = PhysicalRect {
            left: 5000,
            top: 4000,
            right: 5600,
            bottom: 4400,
        };
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };

        assert_eq!(
            restored_window_origin(window, work_area),
            Some(PhysicalPoint { x: 1320, y: 0 }),
        );
    }

    #[test]
    fn partly_visible_window_keeps_a_reachable_title_strip() {
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let window = PhysicalRect {
            left: 1880,
            top: 32,
            right: 2480,
            bottom: 432,
        };

        assert!(native_rect_has_reachable_title_area(window, &[work_area]));
    }

    #[test]
    fn window_below_the_work_area_does_not_count_as_title_reachable() {
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let window = PhysicalRect {
            left: 100,
            top: 1050,
            right: 700,
            bottom: 1450,
        };

        assert!(!native_rect_has_reachable_title_area(window, &[work_area]));
    }

    #[test]
    fn main_host_local_bounds_convert_to_virtual_desktop_physical_origin() {
        let screen = screen_position_from_main_local(
            Point::new(120.0, 56.0),
            Point::new(32.0, 48.0),
            PhysicalPoint { x: 1400, y: 80 },
            1.5,
        ).unwrap();
        assert_eq!(screen, PhysicalPoint { x: 1628, y: 236 });
        assert_eq!(
            main_local_point_from_screen(
                screen,
                PhysicalPoint { x: 1400, y: 80 },
                Point::new(32.0, 48.0),
                1.5,
            ),
            Some(Point::new(120.0, 56.0)),
        );
    }
}

impl Drop for FloatingWindowManager {
    fn drop(&mut self) {
        if self.native_move.is_some_and(|state| state.high_resolution_timer) {
            let _ = self.native_ops.end_high_resolution_timer();
        }
    }
}
