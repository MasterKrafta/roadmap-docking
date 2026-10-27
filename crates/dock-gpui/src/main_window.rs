use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use dock_core::{DockCommand, DragHandle, DragPayload, FloatingGroupId, Point, Rect, Size};
use gpui::{
    Context, DispatchPhase, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, StatefulInteractiveElement,
    Styled, Subscription, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{with_focus_ring, PopupMenu, PopupMenuItem};
use crate::host::DockHost;
use crate::dock::{
    floating_group_at_point, resolve_shortcut, DockController, DockInitializationError, Shortcut,
    WindowHost,
};
use crate::menus::{self, MenuItemModel, MenuState};
use crate::requests::{DockMenuRequest, ResizeHandle};
use crate::DockSurface;

/// Host window for the dock surface. Commands and pointer gestures dispatch through
/// the controller; popup state is transient and owned by the window.
pub(crate) struct MainWindow {
    theme: Theme,
    dock: DockController,
    menu_state: MenuState,
    activation_subscription: Option<Subscription>,
    workspace_path: Option<PathBuf>,
    workspace_snapshot: Option<String>,
    workspace_save_generation: u64,
}

impl MainWindow {
    pub(crate) fn new_with_workspace(
        theme: Theme,
        workspace_path: Option<&Path>,
    ) -> Result<Self, DockInitializationError> {
        let dock = match crate::dock::load_layout(workspace_path, &theme) {
            Some(layout) => match DockController::new_with_layout(&theme, layout) {
                Ok(dock) => dock,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "restored workspace could not be installed; using initial layout"
                    );
                    DockController::new_initial(&theme)?
                }
            },
            None => DockController::new_initial(&theme)?,
        };

        let workspace_path = workspace_path.map(Path::to_path_buf);
        let workspace_snapshot = workspace_path.as_deref().and_then(|_| {
            crate::dock::serialize_layout(dock.layout(), dock.metrics()).ok()
        });

        Ok(Self {
            dock,
            theme,
            menu_state: MenuState::Closed,
            activation_subscription: None,
            workspace_path,
            workspace_snapshot,
            workspace_save_generation: 0,
        })
    }

    /// Debounces persistence only when the serialized persistent workspace changes.
    fn schedule_workspace_save(&mut self, cx: &mut Context<Self>) {
        if self.workspace_path.is_none() {
            return;
        }

        let snapshot = match crate::dock::serialize_layout(
            self.dock.layout(),
            self.dock.metrics(),
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(error = %error, "could not prepare workspace autosave");
                return;
            }
        };
        if self.workspace_snapshot.as_deref() == Some(snapshot.as_str()) {
            return;
        }
        self.workspace_snapshot = Some(snapshot);
        self.workspace_save_generation = self.workspace_save_generation.wrapping_add(1);
        let generation = self.workspace_save_generation;

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |this, _cx| {
                if this.workspace_save_generation == generation {
                    this.save_workspace_now();
                }
            });
        })
        .detach();
    }

    /// Writes the latest persistent state synchronously, used by debounce and close.
    pub(crate) fn save_workspace_now(&mut self) {
        let Some(path) = self.workspace_path.clone() else {
            return;
        };

        match crate::dock::save_layout(
            Some(path.as_path()),
            self.dock.layout(),
            self.dock.metrics(),
        ) {
            Ok(true) => {
                self.workspace_snapshot =
                    crate::dock::serialize_layout(self.dock.layout(), self.dock.metrics()).ok();
            }
            Ok(false) => {}
            Err(error) => {
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

    pub(crate) fn close_floating_window(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) {
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

        if let Err(error) = self.dock.begin_resize(handle, dock_pointer(position), &frame) {
            tracing::warn!(error = ?error, "could not begin dock resize");
            return;
        }

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

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation_subscription.is_none() {
            self.activation_subscription = Some(cx.observe_window_activation(
                window,
                |this, window, cx| {
                    if !window.is_window_active()
                        && (this.dock.cancel_resize() || this.dock.cancel_drag())
                    {
                        cx.notify();
                    }
                },
            ));
        }

        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        let chrome_mode = self.dock.layout().chrome();
        let toolbar_visible = chrome_mode.toolbar_visible();
        let surface_top = self.surface_top();
        let content_height = (height - surface_top).max(1.0);

        // Escape and popup navigation run in capture; global shortcuts run in bubble
        // so focused input controls can consume Tab/Undo/Redo first.
        let key_down = cx.listener::<KeyDownEvent>(|this, event, window, cx| {
            this.handle_key_down(event, window, cx);
        });
        window.on_key_event(
            move |event: &KeyDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    key_down(event, window, cx);
                }
            },
        );
        let global_shortcuts = cx.listener::<KeyDownEvent>(|this, event, window, cx| {
            this.handle_global_shortcuts(event, window, cx);
        });
        window.on_key_event(
            move |event: &KeyDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    global_shortcuts(event, window, cx);
                }
            },
        );

        // Element mouse-move handlers stop receiving events once the pointer leaves
        // the surface hitbox. Keep tracking active pointer gestures at window scope
        // while the pointer is outside the surface.
        let outside_move = cx.listener::<MouseMoveEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_move(event, window, cx);
        });
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase, window, cx| {
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
            },
        );

        let outside_up = cx.listener::<MouseUpEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_up(event, window, cx);
        });
        window.on_mouse_event(
            move |event: &MouseUpEvent, phase, window, cx| {
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
            },
        );

        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));

        let frame = match self.dock.frame(viewport) {
            Ok(frame) => frame,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Could not compute dock geometry: {error:?}"));
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
        let menu_bounds = menu_state_snapshot
            .anchor()
            .map(|anchor| menus::popup_bounds(anchor, menu_viewport, menu_items.len()));

        if let Some(bounds) = menu_bounds {
            let outside_menu_down = cx.listener::<MouseDownEvent>(
                move |this, event, _window, cx| {
                    if event.button == MouseButton::Left
                        && this.menu_state.is_open()
                        && !menus::contains(bounds, dock_pointer(event.position))
                    {
                        this.menu_state = MenuState::Closed;
                        cx.notify();
                    }
                },
            );
            window.on_mouse_event(
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        outside_menu_down(event, window, cx);
                    }
                },
            );
        }

        if let Some(flyout) = frame.rects().flyout {
            let edge_padding = (self.theme.metrics.splitter_hit_zone.min(flyout.rect.size.width))
                / 2.0;
            let flyout_hit_bounds = Rect::new(
                Point::new(flyout.rect.origin.x - edge_padding, flyout.rect.origin.y),
                Size::new(flyout.rect.size.width + edge_padding * 2.0, flyout.rect.size.height),
            );
            let menu_bounds_for_flyout = menu_bounds;
            let outside_flyout_down = cx.listener::<MouseDownEvent>(
                move |this, event, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
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
            window.on_mouse_event(
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        outside_flyout_down(event, window, cx);
                    }
                },
            );
        }

        // Raise floating windows during capture, but do not let clicks on a menu
        // item raise an underlying floating window before the popup handles them.
        let floating_hit_targets = self
            .dock
            .layout()
            .floating_groups_in_hit_test_order()
            .into_iter()
            .filter_map(|(group, _)| {
                frame
                    .rects()
                    .floating_groups
                    .iter()
                    .find(|floating| floating.group == group)
                    .map(|floating| (group, floating.rect))
            })
            .collect::<Vec<_>>();
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
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                raise_floating(event, window, cx);
            }
        });

        for overflow in &frame.rects().vertical_overflow {
            self.dock
                .scroll_handles_mut()
                .entry(overflow.owner)
                .or_insert_with(gpui::ScrollHandle::new);
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
                .child("Dockers ▾"),
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
                        Some(Box::new(cx.listener(
                            move |this, _event, window, cx| {
                                this.activate_menu_item(index, window, cx);
                                cx.notify();
                            },
                        )) as Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static>)
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
        root = root.child(
            DockSurface::new(
                self.dock.layout(),
                &frame,
                self.dock.scroll_handles(),
                &self.theme,
                self.dock.drag_overlay(),
            )
            .build(&mut host),
        );

        if let Some(popup_menu) = popup_menu {
            root.child(popup_menu)
        } else {
            root
        }
    }
}

#[cfg(test)]
mod tests {
    use super::surface_contains_window_point;
    use gpui::{point, px};

    #[test]
    fn surface_contains_top_left_but_excludes_right_and_bottom_edges() {
        let contains = |x, y| {
            surface_contains_window_point(point(px(x), px(y)), 200.0, 24.0, 76.0)
        };

        assert!(contains(0.0, 24.0));
        assert!(contains(199.0, 99.0));
        assert!(!contains(200.0, 30.0));
        assert!(!contains(-1.0, 30.0));
        assert!(!contains(20.0, 23.0));
        assert!(!contains(20.0, 100.0));
    }
}

