//! Runtime owner of the native windows corresponding to Core floating groups.
//!
//! The manager stores only window handles and derived panel ownership. The
//! shared DockController remains the sole owner of Layout and panel views.

use std::{collections::{HashMap, HashSet}, time::Duration};

use dock_core::{
    DockSide, DragPayload, DragState, DropSource, DropTarget, FloatingGroupId,
    PanelId, Point, Rect,
};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, WeakEntity, Window, WindowBounds,
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
    group: Option<FloatingGroupId>,
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
}

/// Reconciles the native-window registry with the current Core Layout.
pub struct FloatingWindowManager {
    controller: Entity<DockController>,
    theme: DockTheme,
    main_native_window: Option<NativeWindowId>,
    main_host_origin: Point,
    native_move: Option<NativeMoveSession>,
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

    pub(crate) fn native_move_active(&self) -> bool { self.native_move.is_some() }

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
            let Some(point) = self.core_point_for_main(cursor) else { state.drop_target = None; return; };
            self.controller.update(cx, |controller, _| {
                let target = controller.prepare_drop_resolver(source).ok()?.resolve(point).ok()?;
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                target.map(|target| NativeMoveTarget { group: None, target, point })
            })
        } else {
            let Some((target_group, target_id)) = self.registry.floating.iter().find_map(|(group, entry)| {
                (entry.native_window == Some(target_window) && *group != state.group).then_some((*group, entry.window_id))
            }) else { state.drop_target = None; return; };
            let Some(point) = self.core_point_for_floating(target_window, cursor) else { state.drop_target = None; return; };
            self.controller.update(cx, |controller, _| {
                let target = controller.prepare_floating_window_drop_resolver(target_id, target_group, source).ok()?.resolve(point).ok()?;
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                target.map(|target| NativeMoveTarget { group: Some(target_group), target, point })
            })
        };
        state.drop_target = resolved;
    }


    fn clear_native_drag_target(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, _| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
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
        let main_drop = state.drop_target.filter(|target| target.group.is_none())
            .map(|target| (target.target, target.point));
        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_tearoff_move_in(state.group, origin, main_drop, controller_cx)
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
        let (mut groups, panel_owners) = self.controller.update(cx, |controller, _| {
            let layout = controller.layout();
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
                });

                if let Some(area) = layout.dock_area(group.area) {
                    let mut panels = HashSet::new();
                    collect_panel_ids(layout, area.root, &mut panels);
                    for panel in panels {
                        panel_owners.insert(panel, PanelWindowOwner::Floating(id));
                    }
                }
            }

            (groups, panel_owners)
        });

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
    }

    fn release_group_window(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.registry.floating.remove(&group) else {
            return;
        };

        if let Some(native_window) = entry.native_window {
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
            entry.z_order = group.z_order;
            if let Some(native_window) = entry.native_window
                && let Err(error) = self.native_ops.show_no_activate(native_window)
            {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not show a reused floating window"
                );
            }
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
            if let Err(error) = self.native_ops.show_no_activate(native_window) {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not show the new floating window"
                );
            }
        }

        Some(FloatingWindowEntry {
            window,
            window_id,
            native_window,
            bounds: group.bounds,
            z_order: group.z_order,
        })
    }

    fn sync_window_bounds(
        &self,
        entry: &mut FloatingWindowEntry,
        group: FloatingGroupSnapshot,
    ) {
        if entry.bounds == group.bounds {
            return;
        }

        if let Some(native_window) = entry.native_window {
            let scale = self
                .native_ops
                .scale_factor(native_window)
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .unwrap_or(1.0);

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

            if entry.bounds.size != group.bounds.size
                && let (Some(width), Some(height)) = (
                    physical_coordinate(group.bounds.size.width, scale),
                    physical_coordinate(group.bounds.size.height, scale),
                )
            {
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

        entry.bounds = group.bounds;
    }
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
    use super::{main_local_point_from_screen, screen_position_from_main_local};
    use dock_core::Point;
    use platform::PhysicalPoint;

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
