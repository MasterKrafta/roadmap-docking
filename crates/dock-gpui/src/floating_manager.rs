//! Runtime owner of the native windows corresponding to Core floating groups.
//!
//! The manager stores only window handles and derived panel ownership. The
//! shared DockController remains the sole owner of Layout and panel views.

use std::collections::{HashMap, HashSet};

use dock_core::{DockSide, FloatingGroupId, PanelId, Rect};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, Window, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};
use platform::{
    NativeWindowId, NativeWindowOps, PhysicalPoint, PhysicalSize, SystemNativeWindowOps,
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
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(group.bounds.origin.x), px(group.bounds.origin.y)),
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

fn schedule_native_move_tick(
    window: &mut Window,
    manager: WeakEntity<FloatingWindowManager>,
    group: FloatingGroupId,
) {
    window.on_next_frame(move |window, app| {
        let keep_moving = manager
            .update(app, |manager, cx| manager.tick_native_move(group, cx))
            .unwrap_or(false);
        if keep_moving {
            schedule_native_move_tick(window, manager, group);
        }
    });
}

fn physical_coordinate(value: f32, scale: f32) -> Option<i32> {
    let scaled = value * scale;
    (scaled.is_finite() && scaled >= i32::MIN as f32 && scaled <= i32::MAX as f32)
        .then(|| scaled.round() as i32)
}
