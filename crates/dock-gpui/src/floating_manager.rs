//! Runtime owner of native windows for floating dock compositions.
//!
//! Main-dock panels do not allocate native surfaces. A composition gets one
//! host window only after it becomes a floating group. The shared DockController
//! remains the sole owner of Layout and panel views.

use std::{collections::{HashMap, HashSet}, time::Duration};

use dock_core::{
    DockCommand, DockSide, DragPayload, DragState, DropSource, DropTarget, FloatingGroupId,
    FloatingResizeEdge, Node, NodeId, PanelId, Point, Rect, Size,
};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowId, WindowKind, WindowOptions, point, px, size,
};
use platform::{
    hide_from_taskbar, NativeKey, NativeWindowAtPoint, NativeWindowId, NativeWindowOps,
    PhysicalPoint, PhysicalRect, PhysicalSize, SystemNativeWindowOps,
};

use crate::{
    DockController, DockEvent, DockTheme, DragHostManager, DragSourceHost,
    components::FloatingHost, drag_host_manager::DragHostSurface, geometry::DockGeometry,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelLayoutLocation {
    Main,
    Floating(FloatingGroupId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelWindowVisibility {
    Hidden,
    Visible,
}

#[derive(Clone, Copy)]
enum PanelWindowHandle {
    Floating(WindowHandle<FloatingHost>),
    DragHost(WindowHandle<DragHostSurface>),
}

impl PanelWindowHandle {
    fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            Self::Floating(window) => window.update(cx, |_, _, _| ()).is_ok(),
            Self::DragHost(window) => window.update(cx, |_, _, _| ()).is_ok(),
        }
    }

    fn set_group(
        &self,
        group: Option<FloatingGroupId>,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |host, window, host_cx| {
                    host.set_host_group(group, window);
                    host_cx.notify();
                })
                .map_err(|error| error.to_string()),
            Self::DragHost(window) => {
                match window.update(cx, |surface, window, host_cx| {
                    let updated = surface.set_floating_group(group, window, host_cx);
                    host_cx.notify();
                    updated
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("DragHost could not switch its Floating composition".into()),
                    Err(error) => Err(error.to_string()),
                }
            }
        }
    }

    /// Requests a redraw from the exact GPUI window that owns this host.
    fn notify_host(&self, cx: &mut Context<FloatingWindowManager>) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
            Self::DragHost(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
        }
    }

    fn remove_window(&self, cx: &mut Context<FloatingWindowManager>) {
        match self {
            Self::Floating(window) => {
                let _ = window.update(cx, |host, window, _| {
                    host.set_host_group(None, window);
                    window.remove_window();
                });
            }
            Self::DragHost(window) => {
                let _ = window.update(cx, |surface, window, surface_cx| {
                    let _ = surface.set_floating_group(None, window, surface_cx);
                    window.remove_window();
                });
            }
        }
    }

    /// Refreshes GPUI's internal viewport after the native HWND was resized directly.
    fn refresh_gpui_bounds(
        &self,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, window, app| window.bounds_changed(app))
                .map_err(|error| error.to_string()),
            Self::DragHost(window) => window
                .update(cx, |_, window, app| window.bounds_changed(app))
                .map_err(|error| error.to_string()),
        }
    }
}

struct PanelWindowEntry {
    window: PanelWindowHandle,
    window_id: WindowId,
    native_window: Option<NativeWindowId>,
    bounds: Rect,
    native_min_size: Option<PhysicalSize>,
    native_scale_factor: Option<f32>,
    resize_enabled: bool,
    host_group: Option<FloatingGroupId>,
    visibility: PanelWindowVisibility,
}

/// Native-window records keyed by the PanelId currently hosting a floating composition.
///
/// Main-dock panels have no native-window record. When a composition is floated,
/// one of its panels is selected as the host until that composition is docked or
/// the selected host panel leaves it.
#[derive(Default)]
pub struct PanelWindowRegistry {
    windows: HashMap<PanelId, PanelWindowEntry>,
    panel_locations: HashMap<PanelId, PanelLayoutLocation>,
}

impl PanelWindowRegistry {
    pub fn panel_count(&self) -> usize {
        self.windows.len()
    }

    pub fn visible_count(&self) -> usize {
        self.windows
            .values()
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .count()
    }

    pub fn window_for_panel(&self, panel: PanelId) -> Option<WindowHandle<FloatingHost>> {
        match self.windows.get(&panel)?.window {
            PanelWindowHandle::Floating(window) => Some(window),
            PanelWindowHandle::DragHost(_) => None,
        }
    }

    pub fn window_id_for_panel(&self, panel: PanelId) -> Option<WindowId> {
        self.windows.get(&panel).map(|entry| entry.window_id)
    }

    pub fn bounds_for_panel(&self, panel: PanelId) -> Option<Rect> {
        self.windows.get(&panel).map(|entry| entry.bounds)
    }

    pub fn native_min_size_for_panel(&self, panel: PanelId) -> Option<PhysicalSize> {
        self.windows.get(&panel)?.native_min_size
    }

    pub fn native_scale_factor_for_panel(&self, panel: PanelId) -> Option<f32> {
        self.windows.get(&panel)?.native_scale_factor
    }

    pub fn resize_enabled_for_panel(&self, panel: PanelId) -> Option<bool> {
        self.windows.get(&panel).map(|entry| entry.resize_enabled)
    }

    pub fn native_window_for_panel(&self, panel: PanelId) -> Option<NativeWindowId> {
        self.windows.get(&panel)?.native_window
    }

    pub fn layout_location_for_panel(&self, panel: PanelId) -> Option<PanelLayoutLocation> {
        self.panel_locations.get(&panel).copied()
    }

    pub fn host_group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.windows.get(&panel)?.host_group
    }

    pub fn visibility_for_panel(&self, panel: PanelId) -> Option<PanelWindowVisibility> {
        self.windows.get(&panel).map(|entry| entry.visibility)
    }
}

/// Maps each Core floating composition to the panel window created on demand
/// to host it. The Main Dock is hosted by the one shared MainWindow.
#[derive(Default)]
pub struct CompositionHostRegistry {
    main_window_id: Option<WindowId>,
    floating_groups: HashMap<FloatingGroupId, PanelId>,
}

impl CompositionHostRegistry {
    pub fn main_window_id(&self) -> Option<WindowId> {
        self.main_window_id
    }

    pub fn panel_for_group(&self, group: FloatingGroupId) -> Option<PanelId> {
        self.floating_groups.get(&group).copied()
    }

    pub fn group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.floating_groups
            .iter()
            .find_map(|(group, host)| (*host == panel).then_some(*group))
    }

    pub fn floating_count(&self) -> usize {
        self.floating_groups.len()
    }
}

/// A registered persistent DockHost under the physical cursor, together with
/// the pointer converted to that host's Core-local logical coordinate system.
/// The point is always target-host-local; it is not the original drag source point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DockingWindowAtPoint {
    ForeignWindow,
    /// The pointer in MainHost-local logical coordinates.
    Main {
        point: Point,
    },
    /// The pointer in this FloatingHost's local logical coordinates.
    Floating {
        group: FloatingGroupId,
        window_id: WindowId,
        point: Point,
    },
}

impl DockingWindowAtPoint {
    pub(crate) const fn role(self) -> Option<crate::DockWindowRole> {
        match self {
            Self::Main { .. } => Some(crate::DockWindowRole::Main),
            Self::Floating { group, .. } => Some(crate::DockWindowRole::Floating { group }),
            Self::ForeignWindow => None,
        }
    }
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
    hide_header: bool,
    grab_offset: PhysicalPoint,
    initial_cursor: PhysicalPoint,
    start_bounds: Rect,
    start_pointer: Point,
    drag_threshold: f32,
    threshold_crossed: bool,
    high_resolution_timer: bool,
    drop_target: Option<NativeMoveTarget>,
    /// Hide the opaque source window while the pointer is over another DockHost,
    /// so the destination composition and its drop feedback remain visible.
    source_hidden: bool,
}

#[derive(Clone)]
struct FloatingGroupSnapshot {
    id: FloatingGroupId,
    bounds: Rect,
    z_order: i32,
    minimum_size: Option<Size>,
    panels: Vec<PanelId>,
}

#[derive(Clone, Copy)]
struct NativeResizeSession {
    group: FloatingGroupId,
    window_id: WindowId,
    transaction_started: bool,
    last_native_bounds: PhysicalRect,
    horizontal_edge: Option<FloatingResizeEdge>,
}

/// Reconciles the native-window registry with the current Core Layout.
pub struct FloatingWindowManager {
    controller: Entity<DockController>,
    theme: DockTheme,
    main_native_window: Option<NativeWindowId>,
    main_host_origin: Point,
    native_move: Option<NativeMoveSession>,
    native_resize: Option<NativeResizeSession>,
    drag_host_manager: Option<WeakEntity<DragHostManager>>,
    reconciling: bool,
    registry: PanelWindowRegistry,
    host_registry: CompositionHostRegistry,
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
            drag_host_manager: None,
            reconciling: false,
            registry: PanelWindowRegistry::default(),
            host_registry: CompositionHostRegistry::default(),
            native_ops: SystemNativeWindowOps,
            controller_subscription: None,
        }
    }

    pub fn registry(&self) -> &PanelWindowRegistry {
        &self.registry
    }

    pub fn host_registry(&self) -> &CompositionHostRegistry {
        &self.host_registry
    }

    /// Connects the temporary transport manager without creating an ownership cycle.
    pub(crate) fn attach_drag_host_manager(
        &mut self,
        manager: WeakEntity<DragHostManager>,
    ) {
        self.drag_host_manager = Some(manager);
    }

    pub(crate) fn drag_host_manager(&self) -> Option<Entity<DragHostManager>> {
        self.drag_host_manager.as_ref()?.upgrade()
    }

    /// Registers an already-open DragHost as the native host for a newly floated DockArea.
    /// The HWND is transferred to the persistent registry instead of opening another window.
    pub(crate) fn adopt_drag_host_window(
        &mut self,
        panel: PanelId,
        group: FloatingGroupId,
        window: WindowHandle<DragHostSurface>,
        window_id: WindowId,
        native_window: NativeWindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.registry.windows.contains_key(&panel) {
            return false;
        }
        let Some(bounds) = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        }) else {
            return false;
        };
        // The DragHost was created before the Floating composition header
        // existed. Start reconciliation from its actual native bounds so the
        // newly reserved header height moves the top edge upward and increases
        // the client height instead of shifting the detached tab downward.
        let current_native_bounds = self
            .core_bounds_from_native_window(native_window)
            .unwrap_or(bounds);

        let configured = window.update(cx, |surface, native, surface_cx| {
            surface.set_floating_group(Some(group), native, surface_cx)
        });
        if !matches!(configured, Ok(true)) {
            return false;
        }

        let scale = self.native_ops.scale_factor(native_window).ok();
        self.registry.windows.insert(
            panel,
            PanelWindowEntry {
                window: PanelWindowHandle::DragHost(window),
                window_id,
                native_window: Some(native_window),
                bounds: current_native_bounds,
                native_min_size: None,
                native_scale_factor: scale,
                resize_enabled: false,
                host_group: Some(group),
                visibility: PanelWindowVisibility::Visible,
            },
        );
        // Preserve the adopted panel as the selected host during the first
        // reconciliation; otherwise the normal first-panel selection may open
        // a second native window for the same composition.
        self.host_registry.floating_groups.insert(group, panel);
        self.reconcile(cx);

        self.host_registry.floating_groups.get(&group) == Some(&panel)
            && self.registry.windows.get(&panel).is_some_and(|entry| {
                entry.host_group == Some(group)
                    && entry.native_window == Some(native_window)
                    && entry.window_id == window_id
            })
    }

    fn panel_window_for_group(&self, group: FloatingGroupId) -> Option<&PanelWindowEntry> {
        let panel = self.host_registry.panel_for_group(group)?;
        self.registry.windows.get(&panel)
    }

    /// Connects the manager to the main GPUI window and the shared controller.
    ///
    /// The main HWND is retained as the native owner for each floating host.
    /// The first reconciliation creates windows only for floating compositions
    /// already present in the initial Layout.
    pub(crate) fn attach_main_window_for_drag_routing(&mut self, window: &mut Window) {
        self.host_registry.main_window_id = Some(window.window_handle().window_id());

        if self.main_native_window.is_none() {
            match self.native_ops.hwnd_of(window) {
                Ok(native_window) => self.main_native_window = Some(native_window),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "main native handle is unavailable; cross-window drag routing is deferred"
                    );
                }
            }
        }
    }

    pub(crate) fn attach_main_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attach_main_window_for_drag_routing(window);

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
        let origin = DockGeometry::screen_position_from_host_local(
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
        let native_window = self.panel_window_for_group(source_group)?.native_window?;
        let client = self.native_ops.client_origin(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        let origin = DockGeometry::screen_position_from_host_local(
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
            .panel_window_for_group(source_group)
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
                .windows
                .values()
                .filter_map(|entry| entry.native_window),
        );

        matches!(
            self.native_ops.window_from_point(cursor, &known, None),
            Ok(NativeWindowAtPoint::Known(target_window)) if target_window != source_window
        )
    }

    pub(crate) fn native_move_active(&self) -> bool { self.native_move.is_some() }
    pub(crate) fn hide_floating_header(&self, group: FloatingGroupId) -> bool {
        self.native_move
            .is_some_and(|state| state.group == group && state.hide_header)
    }



    /// Returns whether an HWND belongs to the persistent Main/Floating hosts.
    ///
    /// DragHost is checked separately because it is a temporary window owned by
    /// DragHostManager rather than part of the persistent window registry.
    pub(crate) fn is_managed_native_window(&self, window: NativeWindowId) -> bool {
        is_managed_foreground_window(
            window,
            self.main_native_window,
            self.registry.windows.values().map(|entry| entry.native_window),
            None,
        )
    }

    /// A GPUI host losing focus is not a cancellation when another managed
    /// window of this application received the OS foreground activation.
    ///
    /// If the platform cannot query the foreground window, preserve the
    /// existing native-move behavior as a conservative fallback.
    pub(crate) fn should_preserve_pointer_interaction_on_focus_out(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        // Native Floating-window moves are tracked independently of GPUI focus.
        // Moving over another managed host can transfer foreground activation;
        // the native move loop must keep running until mouse-up or Escape.
        if self.native_move_active() {
            return true;
        }

        // DockHost can lose focus while the pointer crosses between managed
        // native windows. During a content drag, do not cancel synchronously:
        // DragHost's polling loop debounces transient empty/foreign foreground
        // readings and cancels only after focus stays outside managed hosts.
        let drag_host_active = self.drag_host_manager().is_some_and(|manager| {
            manager.update(cx, |manager, _| manager.active_drag_host_id().is_some())
        });
        if drag_host_active {
            return true;
        }

        let interaction_active = self.controller.update(cx, |controller, _| {
            controller.has_active_pointer_interaction()
        });
        if !interaction_active {
            return false;
        }

        let foreground = match self.native_ops.foreground_window() {
            Ok(Some(window)) => window,
            Ok(None) => return false,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "could not determine the foreground window while handling focus loss"
                );
                return self.native_move_active();
            }
        };

        let drag_host_is_foreground = self.drag_host_manager().is_some_and(|manager| {
            manager.update(cx, |manager, _| {
                manager.active_native_window_id() == Some(foreground)
            })
        });

        self.is_managed_native_window(foreground) || drag_host_is_foreground
    }


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
        let Some(entry) = self.panel_window_for_group(group) else {
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
        let Ok(initial_native_bounds) = self.native_ops.window_rect(native_window) else {
            return;
        };

        self.native_resize = Some(NativeResizeSession {
            group,
            window_id,
            transaction_started: false,
            last_native_bounds: initial_native_bounds,
            horizontal_edge: None,
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

        let native_window = self.panel_window_for_group(group).and_then(|entry| {
            (entry.window_id == window_id).then_some(entry.native_window).flatten()
        });
        let Some(native_window) = native_window else {
            self.finish_native_resize_transaction(state, cx);
            return false;
        };

        if let Ok(native_bounds) = self.native_ops.window_rect(native_window) {
            if state.horizontal_edge.is_none() {
                let left_edge_moved = native_bounds.left != state.last_native_bounds.left;
                let right_edge_moved = native_bounds.right != state.last_native_bounds.right;
                state.horizontal_edge = match (left_edge_moved, right_edge_moved) {
                    (true, false) => Some(FloatingResizeEdge::Left),
                    (false, true) => Some(FloatingResizeEdge::Right),
                    _ => None,
                };
            }
            state.last_native_bounds = native_bounds;
        }

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
                    let scale_factor = self
                        .native_ops
                        .scale_factor(native_window)
                        .ok()
                        .filter(|scale| scale.is_finite() && *scale > 0.0)
                        .unwrap_or(1.0);
                    let command = match state.horizontal_edge {
                        Some(edge) => DockCommand::ResizeFloatingBoundsFromEdge {
                            group,
                            bounds,
                            edge,
                            scale_factor,
                        },
                        None => DockCommand::ResizeFloatingBounds { group, bounds },
                    };
                    let result = self.controller.update(cx, |controller, controller_cx| {
                        controller.dispatch_in(command, controller_cx)
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
        let client_size = self.native_ops.client_size(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        DockGeometry::core_bounds_from_native(rect, client_size, scale)
    }

    /// Starts a thresholded native move from the shared floating DockAreaHeader.
    pub(crate) fn begin_existing_window_move(
        &mut self,
        group: FloatingGroupId,
        pointer: Point,
        cx: &mut Context<Self>,
    ) -> bool {
        self.begin_existing_window_move_with_header(group, pointer, false, cx)
    }

    fn begin_existing_window_move_with_header(
        &mut self,
        group: FloatingGroupId,
        pointer: Point,
        hide_header: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() {
            return false;
        }
        let Some(entry) = self.panel_window_for_group(group) else {
            return false;
        };
        let (Some(native_window), window_id) = (entry.native_window, entry.window_id) else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let Ok(native_rect) = self.native_ops.window_rect(native_window) else {
            return false;
        };
        let start_bounds = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        });
        let Some(start_bounds) = start_bounds else {
            return false;
        };
        let metrics = self.controller.update(cx, |controller, _| controller.metrics());
        let scale = self.native_ops.scale_factor(native_window).ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let hidden_header_offset = if hide_header {
            let Some(offset) = DockGeometry::physical_coordinate(metrics.column_header_height, scale) else {
                return false;
            };
            offset
        } else {
            0
        };
        let grab_origin = PhysicalPoint {
            x: native_rect.left,
            y: native_rect.top.saturating_add(hidden_header_offset),
        };
        let grab_offset = DockGeometry::physical_delta_between(cursor, grab_origin);
        if !self.controller.update(cx, |controller, _| {
            controller.begin_floating_window_move(group, pointer)
        }) {
            return false;
        }
        if hide_header {
            let position = DockGeometry::physical_position_for_grab(cursor, grab_offset);
            if let Err(error) = self.native_ops.move_window(native_window, position) {
                tracing::warn!(?group, %error, "could not align floating window after hiding ColumnHeader");
                self.cancel_controller_move(cx);
                return false;
            }
        }
        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group,
            native_window,
            hide_header,
            grab_offset,
            initial_cursor: cursor,
            start_bounds,
            start_pointer: pointer,
            drag_threshold: metrics.drag_threshold * scale,
            threshold_crossed: false,
            high_resolution_timer,
            drop_target: None,
            source_hidden: false,
        });
        self.schedule_native_move(cx, group, window_id);
        if hide_header {
            cx.notify();
        }
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
        if self.panel_window_for_group(group).is_none_or(|entry| entry.window_id != window_id) {
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
            let Some(delta) = DockGeometry::logical_delta_from_physical(
                PhysicalPoint { x: dx, y: dy },
                scale,
            ) else {
                self.cancel_native_move(state, cx);
                return false;
            };
            let Ok(point) = state.start_pointer.checked_offset(delta.x, delta.y) else {
                self.cancel_native_move(state, cx);
                return false;
            };
            self.controller.update(cx, |controller, _| {
                if matches!(controller.drag().session().state(), DragState::Pressed { .. }) {
                    let _ = controller.drag_mut().session_mut().pointer_move(point);
                }
            });
        }
        let position = DockGeometry::physical_position_for_grab(cursor, state.grab_offset);
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
                DragPayload::FloatingGroup { group } if group == state.group => controller
                    .layout()
                    .floating_group(group)
                    .and_then(|floating| controller.layout().dock_area(floating.area))
                    .map(|area| DropSource::Composition { root: area.root }),
                _ => None,
            }
        });
        let Some(source) = source else {
            self.show_dragged_window(state);
            state.drop_target = None;
            self.clear_native_drag_target(cx);
            return;
        };
        let mut known = Vec::new();
        if let Some(main) = self.main_native_window { known.push(main); }
        known.extend(self.registry.windows.values().filter_map(|entry| entry.native_window));
        let target_window = match self.native_ops.window_from_point(
            cursor,
            &known,
            Some(state.native_window),
        ) {
            Ok(NativeWindowAtPoint::Known(window)) => window,
            Ok(NativeWindowAtPoint::Foreign | NativeWindowAtPoint::Empty) | Err(_) => {
                self.show_dragged_window(state);
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            }
        };
        let resolved = if Some(target_window) == self.main_native_window {
            // Main is rendered behind owned Floating windows. Hide the dragged
            // source while it is over Main so the destination is not obscured.
            self.hide_dragged_window(state);
            let Some(point) = self.core_point_for_main(cursor) else {
                self.show_dragged_window(state);
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
            let Some((target_group, target_id)) = self.host_registry.floating_groups.iter().find_map(|(group, panel)| {
                let entry = self.registry.windows.get(panel)?;
                (entry.native_window == Some(target_window) && *group != state.group)
                    .then_some((*group, entry.window_id))
            }) else {
                self.show_dragged_window(state);
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            // Both compositions are opaque native windows. Hide only the
            // dragged source; keep its native bounds tracking the pointer so
            // it can be restored immediately when the cursor leaves the target.
            self.hide_dragged_window(state);
            let Some(point) = self.core_point_for_floating(target_window, cursor) else {
                self.show_dragged_window(state);
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

    fn hide_dragged_window(&self, state: &mut NativeMoveSession) {
        if state.source_hidden {
            return;
        }
        match self.native_ops.hide(state.native_window) {
            Ok(()) => state.source_hidden = true,
            Err(error) => tracing::debug!(
                ?state.group,
                %error,
                "could not hide the dragged Floating window over a docking target"
            ),
        }
    }

    fn show_dragged_window(&self, state: &mut NativeMoveSession) {
        if !state.source_hidden {
            return;
        }
        match self.native_ops.show_no_activate(state.native_window) {
            Ok(()) => state.source_hidden = false,
            Err(error) => tracing::warn!(
                ?state.group,
                %error,
                "could not restore the dragged Floating window after leaving a docking target"
            ),
        }
    }


    fn clear_native_drag_target(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            controller_cx.notify();
        });
    }

    /// Finds the topmost managed target and converts the screen point into that target's
    /// own Core-local logical coordinates. A foreign topmost window is reported distinctly.
    pub(crate) fn docking_window_at_point(
        &self,
        screen: PhysicalPoint,
        drag_host: NativeWindowId,
        excluded_source_window: Option<WindowId>,
    ) -> Option<DockingWindowAtPoint> {
        let Some(main_window_id) = self.host_registry.main_window_id else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let Some(main_native_window) = self.main_native_window else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let mut known_windows = Vec::with_capacity(self.registry.windows.len() + 1);

        if excluded_source_window != Some(main_window_id) {
            known_windows.push(main_native_window);
        }
        known_windows.extend(self.registry.windows.values().filter_map(|entry| {
            (entry.visibility == PanelWindowVisibility::Visible
                && Some(entry.window_id) != excluded_source_window)
                .then_some(entry.native_window)
                .flatten()
        }));

        // The temporary DragHost is never an input target. The native resolver
        // skips it and continues through the desktop Z-order to a registered host.
        let target_native = match self
            .native_ops
            .window_from_point(screen, &known_windows, Some(drag_host))
        {
            Ok(NativeWindowAtPoint::Known(window)) => window,
            Ok(NativeWindowAtPoint::Foreign) | Err(_) => {
                return Some(DockingWindowAtPoint::ForeignWindow);
            }
            Ok(NativeWindowAtPoint::Empty) => return None,
        };

        if target_native == main_native_window {
            return self
                .core_point_for_main(screen)
                .map(|point| DockingWindowAtPoint::Main { point })
                .or(Some(DockingWindowAtPoint::ForeignWindow));
        }

        let Some(entry) = self.registry.windows.values().find(|entry| {
            entry.visibility == PanelWindowVisibility::Visible
                && entry.native_window == Some(target_native)
        }) else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let (Some(group), Some(point)) = (
            entry.host_group,
            self.core_point_for_floating(target_native, screen),
        ) else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        Some(DockingWindowAtPoint::Floating {
            group,
            window_id: entry.window_id,
            point,
        })
    }

    /// Returns true when the cursor is over a registered FloatingHost above MainHost.
    pub(crate) fn cursor_over_floating_window_from_main(&self) -> bool {
        let Some(main) = self.main_native_window else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let mut known = vec![main];
        known.extend(self.registry.windows.values().filter_map(|entry| {
            (entry.visibility == PanelWindowVisibility::Visible)
                .then_some(entry.native_window)
                .flatten()
        }));
        matches!(
            self.native_ops.window_from_point(cursor, &known, None),
            Ok(NativeWindowAtPoint::Known(window)) if window != main
        )
    }

    /// Converts the screen cursor into the original source host's Core-local logical
    /// coordinates. This can differ from the target point returned by docking_window_at_point.
    pub(crate) fn core_point_for_drag_source(
        &self,
        source_host: DragSourceHost,
        source_window_id: Option<WindowId>,
        screen: PhysicalPoint,
    ) -> Option<Point> {
        match source_host {
            DragSourceHost::Main => {
                if source_window_id.is_some_and(|id| Some(id) != self.host_registry.main_window_id) {
                    return None;
                }
                self.core_point_for_main(screen)
            }
            DragSourceHost::Floating { group } => {
                let entry = self.panel_window_for_group(group)?;
                if source_window_id.is_some_and(|id| id != entry.window_id) {
                    return None;
                }
                self.core_point_for_floating(entry.native_window?, screen)
            }
        }
    }

    /// Converts bounds expressed in the original source host's Core-local coordinates
    /// into physical screen coordinates.
    pub(crate) fn screen_bounds_for_drag_source(
        &self,
        source_host: DragSourceHost,
        bounds: Rect,
    ) -> Option<Rect> {
        match source_host {
            DragSourceHost::Main => self.screen_bounds_from_main_local(bounds),
            DragSourceHost::Floating { group } => {
                self.screen_bounds_from_floating_local(group, bounds)
            }
        }
    }

    pub(crate) fn core_point_for_main(&self, screen: PhysicalPoint) -> Option<Point> {
        let main = self.main_native_window?;
        let client = self.native_ops.client_origin(main).ok()?;
        let scale = self.native_ops.scale_factor(main).ok()?;
        DockGeometry::host_local_point_from_screen(
            screen,
            client,
            self.main_host_origin,
            scale,
        )
    }

    fn core_point_for_floating(&self, window: NativeWindowId, screen: PhysicalPoint) -> Option<Point> {
        let client = self.native_ops.client_origin(window).ok()?;
        let scale = self.native_ops.scale_factor(window).ok()?;
        DockGeometry::host_local_point_from_screen(screen, client, Point::ZERO, scale)
    }

    fn finish_native_move(&mut self, mut state: NativeMoveSession, cx: &mut Context<Self>) {
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }

        // The target may belong to the main window or another floating window.
        // Both have already resolved their own local coordinates into a Core target.
        let drop_target = state.drop_target.map(|target| (target.target, target.point));
        let mut floating_bounds = self.controller.update(cx, |controller, _| {
            controller
                .layout()
                .floating_group(state.group)
                .map(|floating| floating.bounds)
        }).unwrap_or(state.start_bounds);

        // The native move can cross onto a monitor with a different DPI.
        // Capture both the actual outer origin and the current client size at
        // that monitor's scale, rather than preserving a potentially stale
        // logical size from the source monitor.
        if let Some(native_bounds) = self.core_bounds_from_native_window(state.native_window) {
            floating_bounds = native_bounds;
        } else if let Ok(rect) = self.native_ops.window_rect(state.native_window) {
            // Keep the final native position even if client-size/DPI querying
            // failed; the last valid Core size remains a safe fallback.
            floating_bounds.origin = Point::new(rect.left as f32, rect.top as f32);
        }

        self.native_move = None;
        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_floating_window_move_in(
                state.group,
                floating_bounds,
                drop_target,
                controller_cx,
            )
        });
        if let Err(error) = result {
            tracing::warn!(?state.group, %error, "could not commit native floating-window move");
            self.cancel_controller_move(cx);
        }

        // Rejected/no-op drops leave the source floating; show it again.
        // Successful docking removes its Floating root, so reconciliation
        // keeps the now-unused source host hidden.
        let source_still_floating = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(state.group).is_some()
        });
        if source_still_floating {
            self.show_dragged_window(&mut state);
        }

        self.redraw_visible_hosts(cx);
        cx.notify();
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
        if let Some(position) = self.physical_origin_for_bounds(state.start_bounds)
            && let Err(error) = self.native_ops.move_window(state.native_window, position)
        {
            tracing::warn!(?state.group, %error, "could not restore floating window after Escape");
        }
        self.show_dragged_window(&mut state);
        cx.notify();
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
        DockGeometry::physical_point_from_core(bounds.origin)
    }

    /// WindowOptions bounds use GPUI logical screen coordinates while Core
    /// stores physical desktop origins. The main-window scale is only the
    /// creation-time approximation; after the HWND is placed, sync_window_bounds
    /// reads its destination monitor's DPI and corrects the native size.
    fn gpui_origin_for_bounds(&self, bounds: Rect) -> Point {
        let scale = self.main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        DockGeometry::logical_point_from_core_physical(bounds.origin, scale)
            .unwrap_or(Point::ZERO)
    }

    /// Makes the PanelId registry and composition-host map match the current Layout.
    ///
    /// Every open panel keeps its own native window. A floating composition is
    /// rendered by one of those windows; the other panel windows stay hidden.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconciling {
            return;
        }
        self.reconciling = true;

        let (mut groups, panel_locations, recover_initial_bounds, transferred_panels) =
            self.controller.update(cx, |controller, _| {
                let recover_initial_bounds = controller.take_initial_monitor_recovery_pending();
                let transferred_panels = controller
                    .drag()
                    .host_session()
                    .map(|session| session.transferred_panels().to_vec())
                    .unwrap_or_default();
                let layout = controller.layout();
                let metrics = controller.metrics();
                let mut panel_locations = HashMap::new();

                for side in [DockSide::Left, DockSide::Right] {
                    if let Some(area) = layout.main_area(side) {
                        let mut panels = Vec::new();
                        let mut seen = HashSet::new();
                        collect_panel_order(layout, area.root, &mut panels, &mut seen);
                        for panel in panels {
                            panel_locations.insert(panel, PanelLayoutLocation::Main);
                        }
                    }
                }

                let mut groups = Vec::new();
                for (id, group) in layout.floating_groups() {
                    let Some(area) = layout.dock_area(group.area) else {
                        continue;
                    };

                    let mut panels = Vec::new();
                    let mut seen = HashSet::new();
                    collect_panel_order(layout, area.root, &mut panels, &mut seen);
                    for panel in &panels {
                        panel_locations.insert(*panel, PanelLayoutLocation::Floating(id));
                    }

                    groups.push(FloatingGroupSnapshot {
                        id,
                        bounds: group.bounds,
                        z_order: group.z_order,
                        minimum_size: layout.floating_minimum_size(id, &metrics).ok(),
                        panels,
                    });
                }

                (groups, panel_locations, recover_initial_bounds, transferred_panels)
            });

        groups.sort_by_key(|group| group.z_order);
        let previous_hosts = self.host_registry.floating_groups.clone();
        let next_hosts = groups
            .iter()
            .filter_map(|group| {
                let previous = previous_hosts
                    .get(&group.id)
                    .copied()
                    .filter(|panel| group.panels.contains(panel));
                let host = previous.or_else(|| group.panels.first().copied())?;
                Some((group.id, host))
            })
            .collect::<HashMap<_, _>>();
        let desired_panels = next_hosts.values().copied().collect::<HashSet<_>>();

        let monitor_work_areas = if recover_initial_bounds {
            match self.native_ops.monitor_work_areas() {
                Ok(areas) => Some(areas),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "could not enumerate monitor work areas for initial floating-window recovery"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Discard externally closed native handles, then create only the
        // composition hosts selected above. Main-dock panels allocate no windows.
        let unavailable_panels = self
            .registry
            .windows
            .iter()
            .filter_map(|(panel, entry)| {
                (!entry.window.is_available(cx)).then_some(*panel)
            })
            .collect::<Vec<_>>();
        for panel in unavailable_panels {
            self.release_panel_window(panel, cx);
        }

        let missing_panels = desired_panels
            .iter()
            .filter(|panel| !self.registry.windows.contains_key(panel))
            .copied()
            .collect::<Vec<_>>();
        for (index, panel) in missing_panels.into_iter().enumerate() {
            let Some(location) = panel_locations.get(&panel).copied() else {
                continue;
            };
            let bounds = self.initial_panel_window_bounds(panel, location, &groups, index, cx);
            if let Some(entry) = self.create_panel_window(panel, bounds, cx) {
                self.registry.windows.insert(panel, entry);
            } else {
                tracing::error!(
                    ?panel,
                    "could not create a native host window for a floating composition"
                );
            }
        }

        // The selected host remains stable while it belongs to the same
        // composition. New compositions get their first panel as a host.
        let mut next_group_by_panel = HashMap::new();
        for (group, panel) in &next_hosts {
            next_group_by_panel.insert(*panel, *group);
        }

        // First detach old hosts and hide them. This happens before any new
        // window mounts the composition, preventing one AnyView from being
        // hosted by two GPUI windows during a host transfer.
        let detached_panels = self
            .registry
            .windows
            .iter()
            .filter_map(|(panel, entry)| {
                (entry.host_group != next_group_by_panel.get(panel).copied())
                    .then_some(*panel)
                    .filter(|_| entry.host_group.is_some())
            })
            .collect::<Vec<_>>();
        // A failed GPUI detach must not be followed by mounting the same
        // composition in another window. Keep that group without an active host
        // for this pass and retry the detach on the next reconciliation.
        let mut blocked_groups = HashSet::new();
        let mut hide_attempted_panels = HashSet::new();
        for panel in detached_panels {
            if let Some(entry) = self.registry.windows.get_mut(&panel) {
                let previous_group = entry.host_group;
                let detached = match entry.window.set_group(None, cx) {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(
                            ?panel,
                            ?previous_group,
                            %error,
                            "could not detach a panel window; composition host transfer is deferred"
                        );
                        if let Some(group) = previous_group
                            && groups.iter().any(|candidate| candidate.id == group)
                        {
                            blocked_groups.insert(group);
                        }
                        false
                    }
                };

                let hidden = if let Some(native_window) = entry.native_window {
                    hide_attempted_panels.insert(panel);
                    match self.native_ops.hide(native_window) {
                        Ok(()) => true,
                        Err(error) => {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not hide a panel window after it stopped hosting a composition"
                            );
                            false
                        }
                    }
                } else {
                    true
                };
                if hidden {
                    entry.visibility = PanelWindowVisibility::Hidden;
                }

                if let Some(native_window) = entry.native_window {
                    if entry.resize_enabled {
                        if let Err(error) = self.native_ops.disable_resize(native_window) {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not disable native resize after a panel stopped hosting"
                            );
                        }
                        entry.resize_enabled = false;
                    }
                    if entry.native_min_size.is_some() {
                        if let Err(error) = self.native_ops.set_min_size(native_window, None) {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not clear the native minimum size after a panel stopped hosting"
                            );
                        }
                        entry.native_min_size = None;
                    }
                }

                // On a failed detach, retain the old GPUI host association.
                // A failed native hide also retains the previous visibility so
                // the next reconciliation retries it instead of trusting a
                // state that no longer matches the HWND.
                if detached {
                    entry.host_group = None;
                }
            }
        }

        self.registry.panel_locations = panel_locations;

        // Publish only hosts whose GPUI view has actually been assigned. A
        // failed assignment remains absent and is retried by the next pass.
        let mut active_hosts = HashMap::new();
        let mut monitor_corrections = Vec::new();
        for group in &groups {
            if blocked_groups.contains(&group.id) {
                continue;
            }
            let Some(panel) = next_hosts.get(&group.id).copied() else {
                continue;
            };
            let Some(mut entry) = self.registry.windows.remove(&panel) else {
                continue;
            };

            if entry.host_group != Some(group.id) {
                match entry.window.set_group(Some(group.id), cx) {
                    Ok(()) => entry.host_group = Some(group.id),
                    Err(error) => {
                        tracing::warn!(
                            ?panel,
                            ?group.id,
                            %error,
                            "could not assign a floating composition to its native host window"
                        );
                        self.registry.windows.insert(panel, entry);
                        continue;
                    }
                }
            }

            self.sync_window_bounds(&mut entry, group, cx);
            if let Some(areas) = monitor_work_areas.as_deref()
                && let Some(corrected_bounds) =
                    self.restore_native_window_to_monitor(&mut entry, areas)
            {
                monitor_corrections.push((group.id, corrected_bounds));
                entry.bounds = corrected_bounds;
            }
            self.sync_native_resize(&mut entry, group);
            self.registry.windows.insert(panel, entry);
            active_hosts.insert(group.id, panel);
        }
        self.host_registry.floating_groups = active_hosts.clone();

        self.synchronize_native_z_order(&groups, &active_hosts, &transferred_panels);

        for (panel, entry) in &mut self.registry.windows {
            if entry.host_group.is_none()
                && entry.visibility == PanelWindowVisibility::Visible
                && !hide_attempted_panels.contains(panel)
            {
                let hidden = if let Some(native_window) = entry.native_window {
                    match self.native_ops.hide(native_window) {
                        Ok(()) => true,
                        Err(error) => {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not hide a non-host panel window"
                            );
                            false
                        }
                    }
                } else {
                    true
                };
                if hidden {
                    entry.visibility = PanelWindowVisibility::Hidden;
                }
            }
        }

        // A composition host is handed to a surviving PanelWindow before the
        // closed panel's own HWND is destroyed. The old host was detached above;
        // the replacement has already been assigned and shown at this point.
        let removed_panels = self
            .registry
            .windows
            .keys()
            .filter(|panel| !desired_panels.contains(panel))
            .copied()
            .collect::<Vec<_>>();
        for panel in removed_panels {
            self.release_panel_window(panel, cx);
        }

        self.reconciling = false;
        if !monitor_corrections.is_empty() {
            let applied = self.controller.update(cx, |controller, controller_cx| {
                let applied = match controller.apply_initial_monitor_bounds(&monitor_corrections)
                {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "could not apply initial monitor recovery to the Core layout"
                        );
                        false
                    }
                };
                controller.flush_events(controller_cx);
                controller_cx.notify();
                applied
            });
            if applied {
                self.reconcile(cx);
            }
        }
    }

    /// Applies Core's floating z-order to the native windows.
    ///
    /// The snapshots arrive sorted from back to front. Showing each existing
    /// composition with SW_SHOWNOACTIVATE preserves that order without stealing
    /// keyboard focus from the window the user is currently interacting with.
    /// This is the only place that reconciles native floating-window z-order.
    fn synchronize_native_z_order(
        &mut self,
        groups: &[FloatingGroupSnapshot],
        active_hosts: &HashMap<FloatingGroupId, PanelId>,
        transferred_panels: &[PanelId],
    ) {
        for group in groups {
            if self.native_move.is_some_and(|session| session.group == group.id) {
                continue;
            }
            let Some(panel) = active_hosts.get(&group.id).copied() else {
                continue;
            };
            let Some(entry) = self.registry.windows.get_mut(&panel) else {
                continue;
            };
            // A newly-created FloatingHost for a transferred composition stays
            // hidden until DragHost has rendered its empty placeholder and
            // released the cached AnyViews. Already-visible hosts remain visible.
            if entry.visibility == PanelWindowVisibility::Hidden
                && group.panels.iter().any(|panel| transferred_panels.contains(panel))
            {
                continue;
            }
            let Some(native_window) = entry.native_window else {
                continue;
            };

            match self.native_ops.show_no_activate(native_window) {
                Ok(()) => entry.visibility = PanelWindowVisibility::Visible,
                Err(error) => tracing::warn!(
                    ?panel,
                    ?group.id,
                    %error,
                    "could not show the native window hosting a floating composition"
                ),
            }
        }
    }

    /// Shows the persistent FloatingHost only after DragHost has stopped
    /// mounting the transferred AnyViews. This prevents a newly-created host
    /// from presenting an empty frame while DragHost still owns the content.
    pub(crate) fn show_handoff_target(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return false;
        };
        let Some((window, native_window, already_visible)) = self.registry.windows.get(&panel).and_then(|entry| {
            (entry.host_group == Some(group)).then_some((
                entry.window,
                entry.native_window?,
                entry.visibility == PanelWindowVisibility::Visible,
            ))
        }) else {
            return false;
        };
        if !already_visible {
            if let Err(error) = self.native_ops.show_no_activate(native_window) {
                tracing::warn!(
                    ?group,
                    ?panel,
                    %error,
                    "could not show the persistent FloatingHost during DragHost handoff"
                );
                return false;
            }
            if let Some(entry) = self.registry.windows.get_mut(&panel) {
                entry.visibility = PanelWindowVisibility::Visible;
            }
        }

        // Request a redraw now; foreground activation is deferred until
        // DragHost has actually been closed, otherwise Windows can activate
        // its owner (Main) again while destroying the owned DragHost.
        self.request_native_redraw(panel);
        if let Err(error) = window.notify_host(cx) {
            tracing::warn!(
                ?group,
                ?panel,
                %error,
                "could not notify the FloatingHost after panel handoff"
            );
            return false;
        }

        cx.notify();
        true
    }

    /// Requests a native paint for the persistent window hosting this panel.
    pub(crate) fn request_native_redraw(&self, panel: PanelId) {
        if let Some(native_window) = self
            .registry
            .windows
            .get(&panel)
            .and_then(|entry| entry.native_window)
            && let Err(error) = self.native_ops.request_redraw(native_window)
        {
            tracing::debug!(?panel, %error, "could not request a native redraw");
        }
    }

    /// Notifies and invalidates all currently visible floating composition hosts.
    pub(crate) fn redraw_visible_hosts(&self, cx: &mut Context<Self>) {
        let hosts = self
            .registry
            .windows
            .iter()
            .filter(|(_, entry)| {
                entry.host_group.is_some() && entry.visibility == PanelWindowVisibility::Visible
            })
            .map(|(panel, entry)| (*panel, entry.window))
            .collect::<Vec<_>>();

        for (panel, window) in hosts {
            if let Err(error) = window.notify_host(cx) {
                tracing::debug!(?panel, %error, "could not notify a visible FloatingHost");
            }
            self.request_native_redraw(panel);
        }
    }

    /// Activates the persistent destination after its temporary DragHost has closed.
    ///
    /// Closing the owned DragHost can restore Main as the foreground window.
    /// Only reclaim focus from Main; never steal it back after the user has
    /// already moved to another application or another floating window.
    pub(crate) fn activate_handoff_target(
        &self,
        group: FloatingGroupId,
        closing_drag_host: NativeWindowId,
    ) -> bool {
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return false;
        };
        let Some(entry) = self.registry.windows.get(&panel) else {
            return false;
        };
        if entry.host_group != Some(group) || entry.visibility != PanelWindowVisibility::Visible {
            return false;
        }
        let Some(native_window) = entry.native_window else {
            return false;
        };

        match self.native_ops.foreground_window() {
            Ok(Some(foreground)) if foreground == native_window => return true,
            Ok(Some(foreground)) if foreground == closing_drag_host => return false,
            Ok(Some(foreground)) if Some(foreground) != self.main_native_window => {
                // Respect a different app window or another FloatingHost that
                // the user activated after releasing the drag.
                return true;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    ?group,
                    %error,
                    "could not inspect foreground window after DragHost close"
                );
                return false;
            }
        }

        if let Err(error) = self.native_ops.activate(native_window) {
            tracing::warn!(
                ?group,
                ?panel,
                %error,
                "could not activate FloatingHost after DragHost close"
            );
            return false;
        }

        matches!(
            self.native_ops.foreground_window(),
            Ok(Some(foreground)) if foreground == native_window
        )
    }

    /// Returns the transferred panels whose destination FloatingHost is ready
    /// to render. FloatingHost calls this from its render pass before scheduling
    /// a next-frame handoff confirmation.
    pub(crate) fn pending_drag_host_handoff_panels(
        &self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> Option<Vec<PanelId>> {
        let host_panel = self.host_registry.panel_for_group(group)?;
        let entry = self.registry.windows.get(&host_panel)?;
        if entry.window_id != window_id
            || entry.host_group != Some(group)
            || entry.visibility != PanelWindowVisibility::Visible
        {
            return None;
        }
        let drag_host = self.drag_host_manager.as_ref()?.upgrade()?;
        drag_host.update(cx, |manager, _| manager.pending_handoff_panels(group))
    }

    /// Called after the destination FloatingHost has completed a frame with
    /// the transferred PanelViews mounted. Only then is the temporary DragHost
    /// removed from the desktop.
    pub(crate) fn notify_drag_host_handoff_rendered(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
        let Some(host_panel) = self.host_registry.panel_for_group(group) else {
            return;
        };
        let Some(entry) = self.registry.windows.get(&host_panel) else {
            return;
        };
        if entry.window_id != window_id
            || entry.host_group != Some(group)
            || entry.visibility != PanelWindowVisibility::Visible
        {
            return;
        }

        if let Some(drag_host) = self.drag_host_manager.as_ref().and_then(|manager| manager.upgrade()) {
            let _ = drag_host.update(cx, |manager, manager_cx| {
                manager.complete_floating_handoff(group, rendered_panels, manager_cx);
            });
        }

        // The destination frame was rendered before DragHost closed. Queue one
        // more native paint after the source surface stops covering the target.
        self.request_native_redraw(host_panel);
    }

    fn initial_panel_window_bounds(
        &self,
        panel: PanelId,
        location: PanelLayoutLocation,
        groups: &[FloatingGroupSnapshot],
        offset: usize,
        cx: &mut Context<Self>,
    ) -> Rect {
        if let PanelLayoutLocation::Floating(group) = location
            && let Some(group) = groups.iter().find(|candidate| candidate.id == group)
        {
            return group.bounds;
        }

        let (viewport, preferred_size, floating_count) =
            self.controller.update(cx, |controller, _| {
                let preferred_size = controller
                    .layout()
                    .panel_instance(panel)
                    .and_then(|instance| controller.registry().get(&instance.key))
                    .map(|descriptor| descriptor.constraints().preferred_size)
                    .unwrap_or(Size::new(280.0, 220.0));
                (
                    controller.viewport(),
                    preferred_size,
                    controller.layout().floating_groups().count(),
                )
            });

        let cascade = (floating_count.saturating_add(offset).min(7) as f32) * 24.0;
        let max_x =
            (viewport.origin.x + viewport.size.width - preferred_size.width).max(viewport.origin.x);
        let max_y =
            (viewport.origin.y + viewport.size.height - preferred_size.height)
                .max(viewport.origin.y);
        let x = (viewport.origin.x
            + (viewport.size.width - preferred_size.width).max(0.0) * 0.5
            + cascade)
            .min(max_x);
        let y = (viewport.origin.y
            + (viewport.size.height - preferred_size.height).max(0.0) * 0.5
            + cascade)
            .min(max_y);

        Rect::new(Point::new(x, y), preferred_size)
    }

    fn release_panel_window(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        let Some(mut entry) = self.registry.windows.remove(&panel) else {
            self.registry.panel_locations.remove(&panel);
            return;
        };

        if let Some(native_window) = entry.native_window {
            if entry.resize_enabled {
                if let Err(error) = self.native_ops.disable_resize(native_window) {
                    tracing::warn!(
                        ?panel,
                        %error,
                        "could not disable native resize before destroying a panel window"
                    );
                }
                entry.resize_enabled = false;
            }
            if entry.native_min_size.is_some() {
                if let Err(error) = self.native_ops.set_min_size(native_window, None) {
                    tracing::warn!(
                        ?panel,
                        %error,
                        "could not clear the native minimum size before destroying a panel window"
                    );
                }
                entry.native_min_size = None;
            }
            if let Err(error) = self.native_ops.hide(native_window) {
                tracing::debug!(
                    ?panel,
                    %error,
                    "panel window was already hidden or unavailable during release"
                );
            }
        }

        entry.window.remove_window(cx);
    }

    fn create_panel_window(
        &mut self,
        panel: PanelId,
        bounds: Rect,
        cx: &mut Context<Self>,
    ) -> Option<PanelWindowEntry> {
        let controller = self.controller.clone();
        let panel_view_controller = self.controller.clone();
        let theme = self.theme;
        let manager = cx.entity().downgrade();
        let logical_origin = self.gpui_origin_for_bounds(bounds);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(logical_origin.x), px(logical_origin.y)),
                size(px(bounds.size.width), px(bounds.size.height)),
            ))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::Floating,
            is_movable: true,
            app_owns_titlebar_drag: true,
            is_resizable: true,
            is_minimizable: false,
            ..Default::default()
        };

        // The GPUI root is created hidden. reconcile assigns its composition,
        // configures the native surface, and only then makes this window visible.
        let window = match cx.open_window(options, move |window, app| {
            // Materialize every open PanelView before this hidden native
            // surface can be shown. This also creates inactive-tab views once;
            // all later hosts reuse the same PanelId -> AnyView cache.
            panel_view_controller.update(app, |controller, panel_cx| {
                controller.ensure_panel_views(window, panel_cx);
            });
            app.new(|_| FloatingHost::new_panel_window(controller, theme, manager))
        }) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(
                    ?panel,
                    %error,
                    "GPUI could not create a floating PanelWindow"
                );
                return None;
            }
        };

        let window_id = match window.update(cx, |_, window, _| {
            window.window_handle().window_id()
        }) {
            Ok(window_id) => window_id,
            Err(error) => {
                tracing::error!(
                    ?panel,
                    %error,
                    "could not inspect the new PanelWindow identifier"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };
        let native_ops = self.native_ops;
        let native_window = match window.update(cx, move |_, window, _| {
            native_ops.hwnd_of(window)
        }) {
            Ok(Ok(native_window)) => Some(native_window),
            Ok(Err(error)) => {
                tracing::debug!(
                    ?panel,
                    %error,
                    "PanelWindow has no native handle"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    ?panel,
                    %error,
                    "could not inspect a newly created PanelWindow"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };

        if self.main_native_window.is_some() && native_window.is_none() {
            // A hidden Windows floating window without an HWND cannot be shown or bound
            // to the application's native owner; do not register it as usable.
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        }

        if let Some(native_window) = native_window {
            // Each floating host window is owned by MainWindow for its lifetime.
            // Dock placement and composition-host transfers never change its owner.
            let Some(owner) = self.main_native_window else {
                tracing::error!(
                    ?panel,
                    "could not create PanelWindow without the main native owner"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            };
            if let Err(error) = self.native_ops.set_owner(native_window, owner) {
                tracing::error!(
                    ?panel,
                    %error,
                    "could not assign the main window as PanelWindow owner"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
            if !hide_from_taskbar(native_window) {
                tracing::warn!(
                    ?panel,
                    "could not hide the newly created PanelWindow from the taskbar"
                );
            }
            if let Some(position) = self.physical_origin_for_bounds(bounds)
                && let Err(error) = self.native_ops.move_window(native_window, position)
            {
                tracing::warn!(
                    ?panel,
                    %error,
                    "could not place a newly created PanelWindow"
                );
            }
        }

        Some(PanelWindowEntry {
            window: PanelWindowHandle::Floating(window),
            window_id,
            native_window,
            bounds,
            native_min_size: None,
            native_scale_factor: None,
            resize_enabled: false,
            host_group: None,
            visibility: PanelWindowVisibility::Hidden,
        })
    }

    /// Repositions a restored window only when its title area is no longer
    /// reachable on any connected monitor. Returns corrected Core bounds.
    fn restore_native_window_to_monitor(
        &self,
        entry: &mut PanelWindowEntry,
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
        if let Some(desired_size) =
            DockGeometry::physical_size_from_logical(entry.bounds.size, destination_scale)
            && let Err(error) = self.native_ops.resize_window(native_window, desired_size)
        {
            tracing::warn!(
                %error,
                "could not rescale a restored floating window for its destination monitor"
            );
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
        entry: &mut PanelWindowEntry,
        group: &FloatingGroupSnapshot,
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
            DockGeometry::physical_size_from_logical(minimum, scale)
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
        entry: &mut PanelWindowEntry,
        group: &FloatingGroupSnapshot,
        cx: &mut Context<Self>,
    ) {
        let Some(native_window) = entry.native_window else {
            entry.native_scale_factor = None;
            entry.bounds = group.bounds;
            return;
        };

        let Some(scale) = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
        else {
            return;
        };
        let scale_changed = entry.native_scale_factor.is_none_or(|previous| {
            (previous - scale).abs() > 0.001
        });
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
            entry.native_scale_factor = Some(scale);
            return;
        }

        // Apply position and client size in one native operation. For a
        // DockTab tear-off, Core moves the top edge upward and increases the
        // height by the same amount; one SetWindowPos keeps the bottom edge
        // anchored without visibly moving the whole window first.
        let Some(position) = self.physical_origin_for_bounds(group.bounds) else {
            tracing::warn!(
                ?group.id,
                "could not convert floating-window origin to physical coordinates"
            );
            return;
        };
        let Some(initial_size) = DockGeometry::physical_size_from_logical(
            group.bounds.size,
            scale,
        ) else {
            tracing::warn!(
                ?group.id,
                "could not convert floating-window size to physical coordinates"
            );
            return;
        };
        if let Err(error) =
            self.native_ops
                .set_window_bounds(native_window, position, initial_size)
        {
            tracing::warn!(
                ?group.id,
                %error,
                "could not synchronize floating native-window bounds"
            );
            return;
        }

        // The HWND has been resized through the native API rather than
        // Window::resize. Refresh GPUI's cached viewport immediately so an
        // adopted DragHost does not keep rendering at its pre-promotion height.
        if let Err(error) = entry.window.refresh_gpui_bounds(cx) {
            tracing::warn!(
                ?group.id,
                %error,
                "could not refresh GPUI bounds after native floating-window resize"
            );
        }

        // Moving to another monitor can synchronously update the HWND DPI.
        // Correct the client size after the atomic bounds update if necessary.
        let effective_scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0);
        let Some(effective_scale) = effective_scale else {
            // Keep the old cache so a later reconciliation retries the sync.
            return;
        };
        let effective_scale_changed = (scale - effective_scale).abs() > 0.001;

        if effective_scale_changed {
            let Some(desired_size) = DockGeometry::physical_size_from_logical(
                group.bounds.size,
                effective_scale,
            ) else {
                tracing::warn!(
                    ?group.id,
                    "could not convert floating-window size for the destination monitor"
                );
                return;
            };
            if let Err(error) = self.native_ops.resize_window(native_window, desired_size) {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not correct floating-window size after a DPI change"
                );
                return;
            }
        }

        entry.native_scale_factor = Some(effective_scale);
        entry.bounds = group.bounds;
    }
}


fn is_managed_foreground_window(
    foreground: NativeWindowId,
    main_window: Option<NativeWindowId>,
    floating_windows: impl IntoIterator<Item = Option<NativeWindowId>>,
    drag_host: Option<NativeWindowId>,
) -> bool {
    main_window == Some(foreground)
        || floating_windows
            .into_iter()
            .flatten()
            .any(|window| window == foreground)
        || drag_host == Some(foreground)
}

fn collect_panel_order(
    layout: &dock_core::Layout,
    root: NodeId,
    panels: &mut Vec<PanelId>,
    seen: &mut HashSet<PanelId>,
) {
    match layout.node(root) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_panel_order(layout, *child, panels, seen);
            }
        }
        Some(Node::Column(column)) => {
            for group in &column.groups {
                collect_panel_order(layout, *group, panels, seen);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_panel_order(layout, *child, panels, seen);
            }
        }
        Some(Node::Panel(panel)) => {
            if seen.insert(panel.panel_id) {
                panels.push(panel.panel_id);
            }
        }
        None => {}
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

    // Keep windows fully inside the work area when they fit. Larger
    // compositions may extend beyond it, but retain a reachable title strip.
    let (minimum_x, maximum_x) = if width <= area_width {
        (
            work_area.left as i64,
            work_area.right as i64 - width as i64,
        )
    } else {
        (
            work_area.left as i64 - width as i64 + 80,
            work_area.right as i64 - 80,
        )
    };
    let (minimum_y, maximum_y) = if height <= area_height {
        (
            work_area.top as i64,
            work_area.bottom as i64 - height as i64,
        )
    } else {
        (
            work_area.top as i64,
            work_area.bottom as i64 - 40,
        )
    };
    let x = (rect.left as i64).clamp(minimum_x, maximum_x);
    let y = (rect.top as i64).clamp(minimum_y, maximum_y);

    Some(PhysicalPoint {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
    })
}

impl Drop for FloatingWindowManager {
    fn drop(&mut self) {
        if self.native_move.is_some_and(|state| state.high_resolution_timer) {
            let _ = self.native_ops.end_high_resolution_timer();
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use super::is_managed_foreground_window;
    use platform::NativeWindowId;

    fn window(raw: isize) -> NativeWindowId {
        NativeWindowId::from_raw(raw).expect("test HWND must be nonzero")
    }

    #[test]
    fn managed_main_floating_and_drag_hosts_preserve_cross_window_focus() {
        let main = window(1);
        let floating = window(2);
        let drag_host = window(3);

        assert!(is_managed_foreground_window(
            main,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
        assert!(is_managed_foreground_window(
            floating,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
        assert!(is_managed_foreground_window(
            drag_host,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
    }

    #[test]
    fn foreground_outside_all_managed_hosts_is_a_real_focus_loss() {
        let main = window(1);
        let floating = window(2);
        let drag_host = window(3);
        let external = window(4);

        assert!(!is_managed_foreground_window(
            external,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
    }
}

#[cfg(test)]
mod coordinate_tests {
    use super::{native_rect_has_reachable_title_area, restored_window_origin};
    use crate::geometry::DockGeometry;
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
            Some(PhysicalPoint { x: 1320, y: 640 }),
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
            left: 1840,
            top: 32,
            right: 2440,
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
        let screen = DockGeometry::screen_position_from_host_local(
            Point::new(120.0, 56.0),
            Point::new(32.0, 48.0),
            PhysicalPoint { x: 1400, y: 80 },
            1.5,
        ).unwrap();
        assert_eq!(screen, PhysicalPoint { x: 1628, y: 236 });
        assert_eq!(
            DockGeometry::host_local_point_from_screen(
                screen,
                PhysicalPoint { x: 1400, y: 80 },
                Point::new(32.0, 48.0),
                1.5,
            ),
            Some(Point::new(120.0, 56.0)),
        );
    }

    #[test]
    fn main_host_coordinates_round_trip_at_fractional_monitor_dpi() {
        let client_origin = PhysicalPoint { x: -1920, y: 120 };
        let host_origin = Point::new(16.0, 24.0);
        let local = Point::new(113.6, 77.0);
        let scale = 1.25;

        let screen =
            DockGeometry::screen_position_from_host_local(local, host_origin, client_origin, scale).unwrap();
        let restored =
            DockGeometry::host_local_point_from_screen(screen, client_origin, host_origin, scale).unwrap();

        assert!((restored.x - local.x).abs() <= 0.25);
        assert!((restored.y - local.y).abs() <= 0.25);
    }

    #[test]
    fn floating_host_coordinates_use_the_target_windows_client_origin_and_dpi() {
        let client_origin = PhysicalPoint { x: -1920, y: 120 };
        let local = Point::new(120.0, 56.0);
        let scale = 1.5;

        let screen = DockGeometry::screen_position_from_host_local(
            local,
            Point::ZERO,
            client_origin,
            scale,
        ).unwrap();
        assert_eq!(screen, PhysicalPoint { x: -1740, y: 204 });
        assert_eq!(
            DockGeometry::host_local_point_from_screen(
                screen,
                client_origin,
                Point::ZERO,
                scale,
            ),
            Some(local),
        );
    }

    #[test]
    fn screen_coordinate_conversion_rejects_invalid_monitor_scale() {
        assert!(
            DockGeometry::screen_position_from_host_local(
                Point::new(1.0, 1.0),
                Point::ZERO,
                PhysicalPoint::default(),
                0.0,
            )
            .is_none()
        );
        assert!(
            DockGeometry::host_local_point_from_screen(
                PhysicalPoint::default(),
                PhysicalPoint::default(),
                Point::ZERO,
                f32::NAN,
            )
            .is_none()
        );
    }
}

