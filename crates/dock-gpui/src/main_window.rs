use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use dock_core::{Axis, DockCommand, DragHandle, DragPayload, FloatingGroupId, Point, Rect, Size};
use gpui::{
    App, AppContext, Bounds, Context, DispatchPhase, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Render, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, canvas, div, point, px, rgb, size,
};
use ui::Theme;

use crate::DockSurface;
use crate::components::{
    PopupMenu, PopupMenuItem,
    focus_ring::{keyboard_resize_axis_for_floating_edge, keyboard_resize_delta},
    with_focus_ring,
};
use crate::dock::{
    DockController, DockInitializationError, Shortcut, WindowHost, floating_group_at_point,
    resolve_shortcut,
};
use crate::host::DockHost;
use crate::labels::{DOCK_GEOMETRY_ERROR_PREFIX, PANEL_MENU_TRIGGER_LABEL};
use crate::menus::{self, MenuItemModel, MenuState};
use crate::requests::{DockMenuRequest, ResizeHandle};

/// Host window for the dock surface. Commands and pointer gestures dispatch through
/// the controller; popup state is transient and owned by the window.
pub(crate) struct MainWindow {
    theme: Theme,
    dock: DockController,
    menu_state: MenuState,
    activation_subscription: Option<Subscription>,
    floating_windows: HashMap<FloatingGroupId, WindowHandle<crate::floating_window::FloatingWindow>>,
    last_dock_viewport: Option<Rect>,
    last_window_bounds: Option<Bounds<Pixels>>,
    last_viewport_size: Option<gpui::Size<Pixels>>,
    last_surface_top: f32,
    last_floating_sync_version: Option<u64>,
    workspace_path: Option<PathBuf>,
    workspace_save_state: WorkspaceSaveState,
    workspace_save_enabled: bool,
}


pub(crate) struct FloatingWindowSnapshot {
    pub(crate) layout: dock_core::Layout,
    pub(crate) frame: FrameGeometry,
    pub(crate) theme: Theme,
    pub(crate) area: dock_core::DockAreaId,
    pub(crate) bounds: Rect,
}

struct FloatingWindowSpec {
    group: FloatingGroupId,
    title: String,
    bounds: Rect,
    native_bounds: Bounds<Pixels>,
}

#[derive(Default)]
struct WorkspaceSaveState {
    saved_snapshot: Option<String>,
    pending_snapshot: Option<String>,
    generation: u64,
}

impl WorkspaceSaveState {
    fn with_saved_snapshot(saved_snapshot: Option<String>) -> Self {
        Self {
            saved_snapshot,
            ..Self::default()
        }
    }

    fn schedule(&mut self, snapshot: String) -> Option<u64> {
        if self.saved_snapshot.as_deref() == Some(snapshot.as_str()) {
            if self.pending_snapshot.take().is_some() {
                self.generation = self.generation.wrapping_add(1);
            }
            return None;
        }
        if self.pending_snapshot.as_deref() == Some(snapshot.as_str()) {
            return None;
        }

        self.pending_snapshot = Some(snapshot);
        self.generation = self.generation.wrapping_add(1);
        Some(self.generation)
    }

    fn begin_save(&mut self, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.pending_snapshot = None;
        true
    }

    fn finish_success(&mut self, snapshot: Option<String>) {
        if let Some(snapshot) = snapshot {
            self.saved_snapshot = Some(snapshot);
        }
        self.finish_attempt();
    }

    fn finish_failed_save(&mut self) {
        self.finish_attempt();
    }

    fn finish_attempt(&mut self) {
        self.pending_snapshot = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

impl MainWindow {
    pub(crate) fn new_with_workspace(
        theme: Theme,
        workspace_path: Option<&Path>,
    ) -> Result<Self, DockInitializationError> {
        let mut workspace_load_failed = false;
        let mut workspace_loaded = false;
        let dock = match crate::dock::load_layout(workspace_path, &theme) {
            Ok(Some(layout)) => match DockController::new_with_layout(&theme, layout) {
                Ok(dock) => {
                    workspace_loaded = true;
                    dock
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "restored workspace could not be installed; using initial layout"
                    );
                    workspace_load_failed = true;
                    DockController::new_initial(&theme)?
                }
            },
            Ok(None) => DockController::new_initial(&theme)?,
            Err(error) => {
                tracing::warn!(
                    path = workspace_path.map_or_else(
                        || "<disabled>".to_owned(),
                        |path| path.display().to_string()
                    ),
                    error = %error,
                    "workspace could not be loaded; using initial layout without enabling saves"
                );
                workspace_load_failed = true;
                DockController::new_initial(&theme)?
            }
        };

        let workspace_path = workspace_path.map(Path::to_path_buf);
        let saved_snapshot = if workspace_loaded {
            workspace_path
                .as_deref()
                .and_then(|_| crate::dock::serialize_layout(dock.layout(), dock.metrics()).ok())
        } else {
            None
        };

        Ok(Self {
            dock,
            theme,
            menu_state: MenuState::Closed,
            activation_subscription: None,
            floating_windows: HashMap::new(),
            last_dock_viewport: None,
            last_window_bounds: None,
            last_viewport_size: None,
            last_surface_top: 0.0,
            last_floating_sync_version: None,
            workspace_path,
            workspace_save_state: WorkspaceSaveState::with_saved_snapshot(saved_snapshot),
            workspace_save_enabled: !workspace_load_failed,
        })
    }


    pub(crate) fn floating_window_snapshot(
        &self,
        group: FloatingGroupId,
    ) -> Option<FloatingWindowSnapshot> {
        let viewport = self.last_dock_viewport?;
        let layout = self.dock.layout().clone();
        let metrics = *self.dock.metrics();
        let area = layout.floating_group(group)?.area;
        let frame = FrameGeometry::compute(&layout, viewport, &metrics).ok()?;
        let bounds = frame
            .rects()
            .floating_groups
            .iter()
            .find(|candidate| candidate.group == group)?
            .rect;

        Some(FloatingWindowSnapshot {
            layout,
            frame,
            theme: self.theme,
            area,
            bounds,
        })
    }

    fn floating_window_specs(&self) -> Vec<FloatingWindowSpec> {
        let (Some(viewport), Some(parent_bounds), Some(parent_viewport)) = (
            self.last_dock_viewport,
            self.last_window_bounds,
            self.last_viewport_size,
        ) else {
            return Vec::new();
        };
        let Ok(frame) = FrameGeometry::compute(self.dock.layout(), viewport, self.dock.metrics())
        else {
            return Vec::new();
        };

        let chrome_width =
            (parent_bounds.size.width.as_f32() - parent_viewport.width.as_f32()).max(0.0);
        let chrome_height =
            (parent_bounds.size.height.as_f32() - parent_viewport.height.as_f32()).max(0.0);
        let surface_top = self.last_surface_top;
        let layout = self.dock.layout();
        let mut specs = Vec::new();

        for (group, _) in layout.floating_groups_in_paint_order() {
            let Some(bounds) = frame
                .rects()
                .floating_groups
                .iter()
                .find(|candidate| candidate.group == group)
                .map(|candidate| candidate.rect)
            else {
                continue;
            };
            let title = layout
                .floating_group(group)
                .and_then(|floating| layout.dock_area(floating.area))
                .and_then(|area| crate::tree_query::first_panel_in_subtree(layout, area.root))
                .and_then(|panel| layout.panel_instance(panel))
                .map(|instance| crate::panels::panel_title(instance).to_owned())
                .unwrap_or_else(|| "Floating panel".to_owned());

            let native_bounds = Bounds::new(
                point(
                    px(parent_bounds.origin.x.as_f32() + bounds.origin.x),
                    px(parent_bounds.origin.y.as_f32() + surface_top + bounds.origin.y),
                ),
                size(
                    px(bounds.size.width + chrome_width),
                    px(bounds.size.height + chrome_height),
                ),
            );
            specs.push(FloatingWindowSpec {
                group,
                title,
                bounds,
                native_bounds,
            });
        }

        specs
    }

    fn floating_bounds_from_native_window(
        &self,
        native_bounds: Bounds<Pixels>,
        viewport_size: gpui::Size<Pixels>,
    ) -> Option<Rect> {
        let parent_bounds = self.last_window_bounds?;
        let origin = Point::new(
            native_bounds.origin.x.as_f32() - parent_bounds.origin.x.as_f32(),
            native_bounds.origin.y.as_f32()
                - parent_bounds.origin.y.as_f32()
                - self.last_surface_top,
        );
        let bounds = Rect::new(
            origin,
            Size::new(viewport_size.width.as_f32(), viewport_size.height.as_f32()),
        );
        bounds.validate_non_empty().ok()?;
        Some(bounds)
    }

    /// Debounces persistence only when the serialized persistent workspace changes.
    fn schedule_workspace_save(&mut self, cx: &mut Context<Self>) {
        if !self.workspace_save_enabled || self.workspace_path.is_none() {
            return;
        }

        let snapshot = match crate::dock::serialize_layout(self.dock.layout(), self.dock.metrics())
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(error = %error, "could not prepare workspace autosave");
                return;
            }
        };
        let Some(generation) = self.workspace_save_state.schedule(snapshot) else {
            return;
        };

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |this, _cx| {
                if this.workspace_save_state.begin_save(generation) {
                    this.save_workspace_now();
                }
            });
        })
        .detach();
    }

    /// Writes the latest persistent state synchronously, used by debounce and close.
    pub(crate) fn save_workspace_now(&mut self) {
        if !self.workspace_save_enabled {
            return;
        }
        let Some(path) = self.workspace_path.clone() else {
            return;
        };

        match crate::dock::save_layout(
            Some(path.as_path()),
            self.dock.layout(),
            self.dock.metrics(),
        ) {
            Ok(true) => {
                let snapshot =
                    crate::dock::serialize_layout(self.dock.layout(), self.dock.metrics()).ok();
                self.workspace_save_state.finish_success(snapshot);
            }
            Ok(false) => self.workspace_save_state.finish_failed_save(),
            Err(error) => {
                self.workspace_save_state.finish_failed_save();
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "workspace save failed"
                );
            }
        }
    }

    pub(crate) fn apply_dock_command(&mut self, command: DockCommand, cx: &mut Context<Self>) {
        if let Err(error) = self.dock.apply(command) {
            tracing::debug!(error = %error, "dock command was rejected");
        }
        self.schedule_workspace_save(cx);
    }

    pub(crate) fn open_menu(
        &mut self,
        request: DockMenuRequest,
        position: gpui::Point<gpui::Pixels>,
    ) {
        self.menu_state = MenuState::open(
            request,
            Point::new(position.x.as_f32(), position.y.as_f32()),
        );
    }

    fn current_menu_items(&mut self, window: &Window) -> Vec<MenuItemModel> {
        let state = self.menu_state.clone();
        if !state.is_open() {
            return Vec::new();
        }

        let viewport = self.dock_viewport(window);
        match self.dock.frame(viewport) {
            Ok(frame) => menus::items(
                self.dock.layout(),
                &self.theme,
                self.dock.metrics(),
                &frame,
                &state,
            ),
            Err(error) => {
                tracing::warn!(error = ?error, "could not prepare popup menu items");
                Vec::new()
            }
        }
    }

    fn activate_menu_item(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) {
        let command = self
            .current_menu_items(window)
            .get(index)
            .filter(|item| item.enabled)
            .and_then(|item| item.command.clone());
        self.menu_state = MenuState::Closed;

        if let Some(command) = command {
            if let Err(error) = self.dock.apply(command) {
                tracing::warn!(error = %error, "popup menu command was rejected");
            }
            self.schedule_workspace_save(cx);
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.to_ascii_lowercase();
        match key.as_str() {
            "escape" => {
                // Cancel an active gesture first, then close a popup, then dismiss flyout.
                if self.dock.cancel_resize() || self.dock.cancel_drag() {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if self.menu_state.is_open() {
                    self.menu_state = MenuState::Closed;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if self.dock.layout().flyout().is_some()
                    && self.dock.layout().flyout_auto_collapse_enabled()
                {
                    if let Err(error) = self.dock.apply(DockCommand::DismissFlyout) {
                        tracing::debug!(error = %error, "flyout dismissal on Escape was rejected");
                    }
                    self.schedule_workspace_save(cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            "down" | "arrowdown" => {
                if self.menu_state.is_open() {
                    let item_count = self.current_menu_items(window).len();
                    self.menu_state.move_selection(1, item_count);
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            "up" | "arrowup" => {
                if self.menu_state.is_open() {
                    let item_count = self.current_menu_items(window).len();
                    self.menu_state.move_selection(-1, item_count);
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            "enter" => {
                if let Some(index) = self.menu_state.selected_index() {
                    self.activate_menu_item(index, window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn handle_global_shortcuts(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shortcut) = resolve_shortcut(&event.keystroke, self.menu_state.is_open()) else {
            return;
        };

        let result = match shortcut {
            Shortcut::Undo => self.dock.undo().map(|_| ()),
            Shortcut::Redo => self.dock.redo().map(|_| ()),
            Shortcut::ToggleChrome => {
                let mode = self.dock.layout().chrome().after_tab_shortcut();
                self.dock.apply(DockCommand::SetChrome { mode }).map(|_| ())
            }
            Shortcut::ToggleDockChrome => {
                let mode = self.dock.layout().chrome().after_shift_tab_shortcut();
                self.dock.apply(DockCommand::SetChrome { mode }).map(|_| ())
            }
        };
        if let Err(error) = result {
            tracing::debug!(error = %error, shortcut = ?shortcut, "dock keyboard shortcut was rejected");
        }
        self.schedule_workspace_save(cx);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn close_floating_window(&mut self, group: FloatingGroupId, cx: &mut Context<Self>) {
        if let Err(error) = self.dock.close_floating(group) {
            tracing::warn!(group = ?group, error = %error, "could not close floating window");
        }
        self.schedule_workspace_save(cx);
    }

    pub(crate) fn begin_drag(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        position: gpui::Point<gpui::Pixels>,
    ) {
        let pointer = dock_surface_pointer(position, self.surface_top());
        if let Err(error) = self.dock.begin_drag(payload, handle, pointer) {
            tracing::debug!(error = ?error, "could not record dock drag pointer-down");
        }
    }

    pub(crate) fn begin_resize(
        &mut self,
        handle: ResizeHandle,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let frame = match self.dock.frame(self.dock_viewport(window)) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(error = ?error, "could not compute frame to begin dock resize");
                return;
            }
        };

        if let Err(error) = self
            .dock
            .begin_resize(handle, dock_pointer(position), &frame)
        {
            tracing::warn!(error = ?error, "could not begin dock resize");
            return;
        }

        cx.notify();
    }

    /// Resizes a focused splitter by a small keyboard-controlled delta.
    ///
    /// Reuses the normal resize session so Core constraints, preview policy,
    /// and undo history remain identical to pointer-driven resizing.
    pub(crate) fn resize_by_keyboard(
        &mut self,
        handle: ResizeHandle,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let axis = match handle {
            ResizeHandle::Splitter { axis, .. } => axis,
            ResizeHandle::OuterColumn { .. } | ResizeHandle::Flyout { .. } => Axis::X,
            ResizeHandle::FloatingEdge { edge, .. } => {
                let Some(axis) = keyboard_resize_axis_for_floating_edge(&event.keystroke, edge)
                else {
                    return;
                };
                axis
            }
        };
        let Some(delta) = keyboard_resize_delta(&event.keystroke, axis) else {
            return;
        };
        cx.stop_propagation();

        let frame = match self.dock.frame(self.dock_viewport(window)) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::debug!(error = ?error, "could not compute frame for keyboard resize");
                return;
            }
        };

        if let Err(error) = self.dock.begin_resize(handle, Point::ZERO, &frame) {
            tracing::debug!(error = ?error, "could not begin keyboard resize");
            return;
        }

        let pointer = match axis {
            Axis::X => Point::new(delta, 0.0),
            Axis::Y => Point::new(0.0, delta),
        };
        if let Err(error) = self.dock.update_resize(pointer) {
            self.dock.cancel_resize();
            tracing::debug!(error = ?error, "keyboard resize preview was rejected");
            cx.notify();
            return;
        }

        if let Err(error) = self.dock.commit_resize(pointer) {
            tracing::debug!(error = ?error, "keyboard resize commit was rejected");
            cx.notify();
            return;
        }

        self.schedule_workspace_save(cx);
        cx.notify();
    }

    pub(crate) fn handle_surface_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dock.resize_session().is_some() {
            let pointer = dock_pointer(event.position);
            if event.pressed_button != Some(MouseButton::Left) {
                // A move without the left button means the release event was lost.
                // Finish the gesture here so the resize session cannot remain stuck.
                if let Err(error) = self.dock.commit_resize(pointer) {
                    tracing::debug!(error = ?error, "dock resize commit after lost mouse-up was rejected");
                }
                self.schedule_workspace_save(cx);
                cx.notify();
                return;
            }

            if let Err(error) = self.dock.update_resize(pointer) {
                tracing::debug!(error = ?error, "dock resize preview was rejected");
            }

            cx.notify();
            return;
        }

        let pointer = dock_surface_pointer(event.position, self.surface_top());
        if event.pressed_button != Some(MouseButton::Left) {
            // The release event may have been lost while a drag was active.
            if self.dock.cancel_drag() {
                cx.notify();
            }
            return;
        }

        let viewport = self.dock_viewport(window);
        let frame = if self.dock.drag_needs_start(pointer) {
            match self.dock.frame(viewport) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    tracing::warn!(error = ?error, "could not compute geometry to start dock drag");
                    if self.dock.cancel_drag() {
                        cx.notify();
                    }
                    return;
                }
            }
        } else {
            None
        };

        match self.dock.update_drag(pointer, viewport, frame.as_ref()) {
            Ok(changed) => {
                if changed {
                    cx.notify();
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "dock drag pointer update failed");
                self.dock.cancel_drag();
                // update_drag may already have cancelled a stale session internally.
                // Always redraw so a removed overlay cannot remain on screen.
                cx.notify();
            }
        }
    }

    pub(crate) fn handle_surface_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        if self.dock.resize_session().is_none() {
            let had_overlay = self.dock.drag_overlay().is_some();
            let pointer = dock_surface_pointer(event.position, self.surface_top());
            let mut changed = had_overlay;
            let viewport = self.dock_viewport(window);
            match self.dock.finish_drag(pointer, viewport) {
                Ok(Some(request)) => {
                    changed = true;
                    match self.dock.commit_drop_request(request, viewport) {
                        Ok(action) => {
                            tracing::debug!(action = ?action, "dock drag request resolved and committed");
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "dock drag request was rejected");
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    changed = true;
                    tracing::warn!(error = ?error, "could not finish dock drag");
                }
            }
            self.schedule_workspace_save(cx);
            // Also clears the active overlay when a drag ends without a valid target.
            if changed {
                cx.notify();
            }
            return;
        }

        if let Err(error) = self.dock.commit_resize(dock_pointer(event.position)) {
            tracing::debug!(error = ?error, "dock resize commit was rejected");
        }
        self.schedule_workspace_save(cx);

        cx.notify();
    }

    fn surface_top(&self) -> f32 {
        if self.dock.layout().chrome().toolbar_visible() {
            self.theme.metrics.column_header_height.max(1.0)
        } else {
            0.0
        }
    }

    fn dock_viewport(&self, window: &Window) -> Rect {
        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let surface_top = self.surface_top();
        Rect::new(
            Point::ZERO,
            Size::new(width, (height - surface_top).max(1.0)),
        )
    }
}

fn dock_pointer(position: gpui::Point<gpui::Pixels>) -> Point {
    Point::new(position.x.as_f32(), position.y.as_f32())
}

/// Converts window coordinates to the DockSurface's local geometry coordinates.
fn dock_surface_pointer(position: gpui::Point<gpui::Pixels>, surface_top: f32) -> Point {
    let pointer = dock_pointer(position);
    Point::new(pointer.x, pointer.y - surface_top)
}

fn surface_contains_window_point(
    position: gpui::Point<gpui::Pixels>,
    width: f32,
    top: f32,
    height: f32,
) -> bool {
    let x = position.x.as_f32();
    let y = position.y.as_f32();
    x >= 0.0 && x < width && y >= top && y < top + height
}

impl MainWindow {
    /// Registers window-level pointer and keyboard listeners during paint.
    fn window_event_handlers(
        cx: &mut Context<Self>,
        width: f32,
        surface_top: f32,
        content_height: f32,
        menu_bounds: Option<Rect>,
        flyout_hit_bounds: Option<Rect>,
        floating_hit_targets: Vec<(FloatingGroupId, Rect)>,
    ) -> impl IntoElement + use<> {
        // Window-level listeners must be registered during paint, not render.
        // Escape and popup navigation run in capture; global shortcuts run in bubble
        // so focused input controls can consume Tab/Undo/Redo first.
        let key_down = cx.listener::<KeyDownEvent>(|this, event, window, cx| {
            this.handle_key_down(event, window, cx);
        });
        let global_shortcuts = cx.listener::<KeyDownEvent>(|this, event, window, cx| {
            this.handle_global_shortcuts(event, window, cx);
        });

        // Element mouse-move handlers stop receiving events once the pointer leaves
        // the surface hitbox. Keep tracking active pointer gestures at window scope
        // while the pointer is outside the surface.
        let outside_move = cx.listener::<MouseMoveEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_move(event, window, cx);
        });
        let outside_up = cx.listener::<MouseUpEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_up(event, window, cx);
        });

        let outside_menu_down = cx.listener::<MouseDownEvent>(move |this, event, _window, cx| {
            if event.button != MouseButton::Left || !this.menu_state.is_open() {
                return;
            }
            if menu_bounds
                .is_some_and(|bounds| !menus::contains(bounds, dock_pointer(event.position)))
            {
                this.menu_state = MenuState::Closed;
                cx.notify();
            }
        });

        let menu_bounds_for_flyout = menu_bounds;
        let outside_flyout_down = cx.listener::<MouseDownEvent>(
            move |this, event, _window, cx| {
                if event.button != MouseButton::Left {
                    return;
                }
                let Some(flyout_hit_bounds) = flyout_hit_bounds else {
                    return;
                };
                let pointer = dock_surface_pointer(event.position, surface_top);
                let inside_menu = menu_bounds_for_flyout
                    .is_some_and(|bounds| menus::contains(bounds, dock_pointer(event.position)));
                if !inside_menu
                    && !menus::contains(flyout_hit_bounds, pointer)
                    && this.dock.layout().flyout().is_some()
                    && this.dock.layout().flyout_auto_collapse_enabled()
                {
                    if let Err(error) = this.dock.apply(DockCommand::DismissFlyout) {
                        tracing::debug!(error = %error, "flyout dismissal on outside click was rejected");
                    }
                    cx.notify();
                }
            },
        );

        let menu_bounds_for_raise = menu_bounds;
        let raise_floating = cx.listener::<MouseDownEvent>(move |this, event, _window, cx| {
            if event.button != MouseButton::Left
                || menu_bounds_for_raise
                    .is_some_and(|bounds| menus::contains(bounds, dock_pointer(event.position)))
            {
                return;
            }
            let pointer = dock_surface_pointer(event.position, surface_top);
            if let Some(group) = floating_group_at_point(&floating_hit_targets, pointer) {
                this.apply_dock_command(DockCommand::RaiseFloating { group }, cx);
                cx.notify();
            }
        });

        canvas(
            |_, _, _| (),
            move |_, _, window, _cx| {
                window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        key_down(event, window, cx);
                    }
                });
                window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        global_shortcuts(event, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture
                        && !surface_contains_window_point(
                            event.position,
                            width,
                            surface_top,
                            content_height,
                        )
                    {
                        outside_move(event, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture
                        && event.button == MouseButton::Left
                        && !surface_contains_window_point(
                            event.position,
                            width,
                            surface_top,
                            content_height,
                        )
                    {
                        outside_up(event, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        outside_menu_down(event, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        outside_flyout_down(event, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        raise_floating(event, window, cx);
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation_subscription.is_none() {
            self.activation_subscription =
                Some(cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active()
                        && (this.dock.cancel_resize() || this.dock.cancel_drag())
                    {
                        cx.notify();
                    }
                }));
        }

        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        let chrome_mode = self.dock.layout().chrome();
        let toolbar_visible = chrome_mode.toolbar_visible();
        let surface_top = self.surface_top();
        let content_height = (height - surface_top).max(1.0);

        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));
        self.last_dock_viewport = Some(viewport);
        self.last_window_bounds = Some(window.window_bounds().get_bounds());
        self.last_viewport_size = Some(window_size);
        self.last_surface_top = surface_top;

        let frame = match self.dock.frame(viewport) {
            Ok(frame) => frame,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("{DOCK_GEOMETRY_ERROR_PREFIX}: {error:?}"));
            }
        };
        let menu_state_snapshot = self.menu_state.clone();
        let menu_items = menus::items(
            self.dock.layout(),
            &self.theme,
            self.dock.metrics(),
            &frame,
            &menu_state_snapshot,
        );
        let menu_viewport = Rect::new(Point::ZERO, Size::new(width, height));
        let menu_bounds = menu_state_snapshot.anchor().map(|anchor| {
            menus::popup_bounds(anchor, menu_viewport, menu_items.len(), &self.theme)
        });

        let flyout_hit_bounds = frame.rects().flyout.map(|flyout| {
            let edge_padding = (self
                .theme
                .metrics
                .splitter_hit_zone
                .min(flyout.rect.size.width))
                / 2.0;
            Rect::new(
                Point::new(flyout.rect.origin.x - edge_padding, flyout.rect.origin.y),
                Size::new(
                    flyout.rect.size.width + edge_padding * 2.0,
                    flyout.rect.size.height,
                ),
            )
        });
        // Raise floating windows during capture, but do not let clicks on a menu
        // item raise an underlying floating window before the popup handles them.
        let floating_hit_targets = self
            .dock
            .layout()
            .floating_groups_in_hit_test_order()
            .into_iter()
            .filter_map(|(group, _)| {
                if self.floating_windows.contains_key(&group) {
                    return None;
                }
                frame
                    .rects()
                    .floating_groups
                    .iter()
                    .find(|floating| floating.group == group)
                    .map(|floating| (group, floating.rect))
            })
            .collect::<Vec<_>>();
        let global_event_handlers = Self::window_event_handlers(
            cx,
            width,
            surface_top,
            content_height,
            menu_bounds,
            flyout_hit_bounds,
            floating_hit_targets,
        );

        for overflow in &frame.rects().vertical_overflow {
            self.dock
                .scroll_handles_mut()
                .entry(overflow.owner)
                .or_default();
        }

        let mut host = WindowHost::new(cx);
        let dockers_trigger = with_focus_ring(
            div()
                .id("dockers-menu-trigger")
                .flex()
                .items_center()
                .h(px(menu_height))
                .px(px(10.0))
                .text_color(rgb(self.theme.colors.primary_text))
                .child(PANEL_MENU_TRIGGER_LABEL),
            &self.theme,
        )
        .on_click(host.menu(DockMenuRequest::Dockers));

        let popup_menu = menu_bounds.map(|bounds| {
            let selected_index = menu_state_snapshot.selected_index().unwrap_or(0);
            let popup_items = menu_items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let on_select = if item.enabled {
                        Some(Box::new(cx.listener(move |this, _event, window, cx| {
                            this.activate_menu_item(index, window, cx);
                            cx.notify();
                        }))
                            as Box<
                                dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
                            >)
                    } else {
                        None
                    };
                    PopupMenuItem {
                        id: item.id,
                        label: item.label,
                        checked: item.checked,
                        enabled: item.enabled,
                        selected: index == selected_index,
                        on_select,
                    }
                })
                .collect();
            PopupMenu::new(bounds, &self.theme, popup_items)
        });

        let mut root = div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background));

        if toolbar_visible {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(dockers_trigger),
            );
        }
        root = root.child(global_event_handlers);
        root = root.child(
            DockSurface::new(
                self.dock.layout(),
                &frame,
                self.dock.scroll_handles(),
                &self.theme,
                self.dock.drag_overlay(),
            )
            .exclude_floating_groups(
                &self.floating_windows.keys().copied().collect::<HashSet<_>>(),
            )
            .build(&mut host),
        );

        let layout_version = self.dock.layout().version();
        if self.last_floating_sync_version != Some(layout_version) {
            self.last_floating_sync_version = Some(layout_version);
            let owner = cx.entity().downgrade();
            window.defer(cx, move |_, app| {
                synchronize_floating_windows(owner, app);
            });
        }

        if let Some(popup_menu) = popup_menu {
            root.child(popup_menu)
        } else {
            root
        }
    }
}


fn synchronize_floating_windows(
    owner: gpui::WeakEntity<MainWindow>,
    app: &mut App,
) {
    let Some(owner_entity) = owner.upgrade() else {
        return;
    };
    let (specs, existing) = owner_entity.update(app, |main, _cx| {
        (
            main.floating_window_specs(),
            main.floating_windows
                .iter()
                .map(|(group, handle)| (*group, *handle))
                .collect::<Vec<_>>(),
        )
    });

    for (group, handle) in existing {
        let target_bounds = specs
            .iter()
            .find(|spec| spec.group == group)
            .map(|spec| spec.bounds);
        let current_bounds = handle.read(app).ok().map(|view| view.layout_bounds());
        let keep_window = target_bounds
            .zip(current_bounds)
            .is_some_and(|(target, current)| floating_bounds_match(target, current));
        if keep_window {
            continue;
        }

        if let Err(error) = handle.update(app, |_, window, _cx| window.remove_window()) {
            tracing::debug!(
                group = ?group,
                error = %error,
                "could not retire stale native floating window"
            );
        }
        owner_entity.update(app, |main, cx| {
            if main.floating_windows.remove(&group).is_some() {
                cx.notify();
            }
        });
    }

    for spec in specs {
        if owner_entity
            .read(app)
            .floating_windows
            .contains_key(&spec.group)
        {
            continue;
        }

        let group = spec.group;
        let initial_bounds = spec.bounds;
        let title = spec.title.clone();
        let native_bounds = spec.native_bounds;
        let owner_for_view = owner.clone();
        let owner_for_close = owner.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(native_bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(180.0), px(120.0))),
            ..Default::default()
        };
        match app.open_window(options, move |window, cx| {
            let view = cx.new(|_| {
                crate::floating_window::FloatingWindow::new(
                    owner_for_view.clone(),
                    group,
                    initial_bounds,
                )
            });
            view.update(cx, |_, view_cx| {
                window.on_window_should_close(view_cx, move |_window, app| {
                    if let Some(owner) = owner_for_close.upgrade() {
                        owner.update(app, |main, cx| {
                            main.close_floating_window(group, cx);
                            cx.notify();
                        });
                    }
                    true
                });
            });
            view
        }) {
            Ok(handle) => {
                let still_exists = owner_entity
                    .read(app)
                    .dock
                    .layout()
                    .floating_group(group)
                    .is_some();
                if still_exists {
                    owner_entity.update(app, |main, cx| {
                        if !main.floating_windows.contains_key(&group) {
                            main.floating_windows.insert(group, handle);
                            cx.notify();
                        }
                    });
                } else if let Err(error) =
                    handle.update(app, |_, window, _cx| window.remove_window())
                {
                    tracing::debug!(
                        group = ?group,
                        error = %error,
                        "could not close native window for a removed floating group"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    group = ?group,
                    error = %error,
                    "could not open native floating window"
                );
            }
        }
    }
}

fn floating_bounds_match(left: Rect, right: Rect) -> bool {
    const EPSILON: f32 = 1.0;
    (left.origin.x - right.origin.x).abs() < EPSILON
        && (left.origin.y - right.origin.y).abs() < EPSILON
        && (left.size.width - right.size.width).abs() < EPSILON
        && (left.size.height - right.size.height).abs() < EPSILON
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use gpui::{point, px};
    use ui::Theme;

    use super::{MainWindow, WorkspaceSaveState, surface_contains_window_point};

    fn temporary_workspace_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "roadmap-docking-invalid-workspace-{}-{nonce}.json",
            std::process::id()
        ))
    }

    #[test]
    fn surface_contains_top_left_but_excludes_right_and_bottom_edges() {
        let contains = |x, y| surface_contains_window_point(point(px(x), px(y)), 200.0, 24.0, 76.0);

        assert!(contains(0.0, 24.0));
        assert!(contains(199.0, 99.0));
        assert!(!contains(200.0, 30.0));
        assert!(!contains(-1.0, 30.0));
        assert!(!contains(20.0, 23.0));
        assert!(!contains(20.0, 100.0));
    }

    #[test]
    fn failed_workspace_save_does_not_suppress_retry_for_same_snapshot() {
        let mut state = WorkspaceSaveState::default();
        let generation = state
            .schedule("changed layout".to_owned())
            .expect("a changed layout should schedule a save");

        assert!(state.begin_save(generation));
        state.finish_failed_save();

        let retry_generation = state
            .schedule("changed layout".to_owned())
            .expect("the same layout should be schedulable after a failed save");
        assert_ne!(generation, retry_generation);
    }

    #[test]
    fn successfully_saved_snapshot_does_not_schedule_again() {
        let mut state = WorkspaceSaveState::with_saved_snapshot(Some("saved layout".to_owned()));

        assert!(state.schedule("saved layout".to_owned()).is_none());
    }

    #[test]
    fn newer_snapshot_supersedes_pending_save() {
        let mut state = WorkspaceSaveState::default();
        let old_generation = state
            .schedule("older layout".to_owned())
            .expect("first layout should schedule a save");
        let new_generation = state
            .schedule("newer layout".to_owned())
            .expect("different layout should replace the pending save");

        assert!(!state.begin_save(old_generation));
        assert!(state.begin_save(new_generation));
    }

    #[test]
    fn missing_workspace_can_be_created_when_window_saves() {
        let path = temporary_workspace_path();
        let mut window = MainWindow::new_with_workspace(Theme::default(), Some(&path))
            .expect("the window should open with an initial layout");

        window.save_workspace_now();

        let actual = fs::read_to_string(&path).expect("first-run workspace should be created");
        fs::remove_file(&path).expect("test workspace should be removable");
        assert!(
            actual.contains("\"version\": 2"),
            "a first-run save should write the current workspace schema"
        );
    }

    #[test]
    fn unsupported_workspace_is_not_overwritten_when_window_saves_on_close() {
        let path = temporary_workspace_path();
        let original = r#"{"format":"roadmap-docking-workspace","version":999,"layout":{}}"#;
        fs::write(&path, original).expect("test should be able to write a workspace");

        let mut window = MainWindow::new_with_workspace(Theme::default(), Some(&path))
            .expect("the window should fall back to an initial layout");

        // The close handler calls this method synchronously.
        window.save_workspace_now();

        let actual =
            fs::read_to_string(&path).expect("the original workspace file should still exist");
        fs::remove_file(&path).expect("test workspace should be removable");

        assert_eq!(
            actual, original,
            "a workspace rejected at startup must not be overwritten by the fallback layout"
        );
    }
}
