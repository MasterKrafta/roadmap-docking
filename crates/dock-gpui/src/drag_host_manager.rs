//! Runtime owner of temporary native windows used to present dragged dock content.
//!
//! A DragHost is deliberately independent from Core Layout and the persistent
//! FloatingWindowManager. It hosts the existing source presentation and routes
//! drop feedback to the persistent Main/Floating window under the cursor.

use std::{collections::{HashMap, HashSet}, rc::Rc, time::{Duration, Instant}};

use gpui::{
    AnyView, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, Window,
    WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions, div, point, px, rgb, size,
};
use platform::{NativeError, NativeKey, NativeWindowId, NativeWindowOps, SystemNativeWindowOps};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockCommand, DockMetrics, DockSide, DragDropAction,
    DragHandle, DragPayload, DragState, DropTarget, GroupRect, Node, NodeId, PanelId, PanelRect,
    Point, Rect, Size, SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};

use crate::{
    components::{context_menu::GroupMenuToggle, MainDockView, PanelHost},
    dock_components::{
        collect_group_presentations, DockAreaView, DockCompositionView, DockGroupView,
        GroupPresentation,
    },
    floating_manager::DockingWindowAtPoint,
    geometry::DockGeometry,
    DockController, DockTheme, DockWindowRole, DragHostId, DragSourceHost, FloatingWindowManager,
};

struct ActiveDragHost {
    id: DragHostId,
    window: WindowHandle<DragHostSurface>,
    window_id: WindowId,
    native_window: NativeWindowId,
    panels: Vec<PanelId>,
    grab_offset: Point,
    visible: bool,
    drop_target: Option<DropTarget>,
    target_role: Option<DockWindowRole>,
    free_area_bounds: Option<Rect>,
    /// Cursor position captured when the button is released before source detachment.
    pending_release_cursor: Option<platform::PhysicalPoint>,
    focus_loss_since: Option<Instant>,
    source_detach_subscription: Option<Subscription>,
    handoff: Option<DragHostHandoff>,
}

#[derive(Clone, Copy)]
struct DragHostHandoff {
    target_group: FloatingGroupId,
    source_placeholder_rendered: bool,
    source_released: bool,
}

/// Creates, positions, and closes the single temporary DragHost allowed during
/// one pointer interaction.
///
/// The manager is owned by DockHost, alongside FloatingWindowManager. It uses
/// the shared controller to transfer an existing cached PanelView, but never
/// creates views or reads/mutates Core Layout directly.
pub struct DragHostManager {
    controller: Entity<DockController>,
    theme: DockTheme,
    main_native_window: Option<NativeWindowId>,
    floating_manager: Option<Entity<FloatingWindowManager>>,
    active: Option<ActiveDragHost>,
    next_id: u64,
    native_ops: SystemNativeWindowOps,
}

impl DragHostManager {
    pub fn new(controller: Entity<DockController>, theme: DockTheme) -> Self {
        Self {
            controller,
            theme,
            main_native_window: None,
            floating_manager: None,
            active: None,
            next_id: 1,
            native_ops: SystemNativeWindowOps,
        }
    }

    /// Returns the shared controller entity. The manager does not own its Layout.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Attaches the persistent-host registry used to route DragHost drop feedback.
    pub(crate) fn attach_floating_window_manager(
        &mut self,
        manager: Entity<FloatingWindowManager>,
    ) {
        self.floating_manager = Some(manager);
    }

    /// Connects the manager to the application's main native window.
    ///
    /// DragHost windows use MainHost as their native owner for application
    /// lifetime and taskbar behavior. Returns false when GPUI cannot provide
    /// the main HWND yet; DockHost may retry on a later render.
    pub(crate) fn attach_main_window(&mut self, window: &mut Window) -> bool {
        if self.main_native_window.is_some() {
            return true;
        }

        match self.native_ops.hwnd_of(window) {
            Ok(native_window) => {
                self.main_native_window = Some(native_window);
                true
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    "main native handle is unavailable; DragHost creation is deferred"
                );
                false
            }
        }
    }

    /// Creates one empty DragHost at the current cursor position.
    ///
    /// `grab_offset` is measured in DockCore logical pixels from the DragHost's
    /// top-left corner to the pointer. The new HWND is created hidden, assigned
    /// to MainHost, positioned, and sized before it is shown without activation.
    pub fn open_at_cursor(
        &mut self,
        grab_offset: Point,
        logical_size: Size,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        self.open_at_cursor_with_content(
            grab_offset,
            logical_size,
            Vec::new(),
            None,
            None,
            None,
            cx,
        )
    }

    /// Opens DragHost with the existing cached view for one active Panel drag.
    ///
    /// The source host stops mounting that Panel before the new surface is shown.
    /// Its unchanged Core rectangle is rendered there as a placeholder.
    pub fn open_panel_at_cursor(
        &mut self,
        panel: PanelId,
        grab_offset: Point,
        logical_size: Size,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        let can_transfer = self.controller.update(cx, |controller, _| {
            controller.can_begin_panel_view_transfer(panel)
        });
        if !can_transfer {
            return Err(DragHostError::PanelTransferUnavailable(panel));
        }
        self.open_at_cursor_with_content(
            grab_offset,
            logical_size,
            vec![panel],
            None,
            None,
            None,
            cx,
        )
    }

    /// Opens DragHost with the source Group's original tabs and active cached PanelView.
    pub fn open_group_at_cursor(
        &mut self,
        group: NodeId,
        panel: PanelId,
        grab_offset: Point,
        logical_size: Size,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        let can_transfer = self.controller.update(cx, |controller, _| {
            controller.can_begin_group_view_transfer(group, panel)
        });
        if !can_transfer {
            return Err(DragHostError::PanelTransferUnavailable(panel));
        }
        self.open_at_cursor_with_content(
            grab_offset,
            logical_size,
            vec![panel],
            Some(group),
            None,
            None,
            cx,
        )
    }

    /// Opens DragHost with every active Group and PanelView in one Column.
    pub fn open_column_at_cursor(
        &mut self,
        column: NodeId,
        panels: Vec<PanelId>,
        grab_offset: Point,
        logical_size: Size,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        let can_transfer = self.controller.update(cx, |controller, _| {
            controller.can_begin_column_view_transfer(column, &panels)
        });
        if !can_transfer {
            return Err(panels.first().copied().map_or(
                DragHostError::ContentTransferUnavailable,
                DragHostError::PanelTransferUnavailable,
            ));
        }
        self.open_at_cursor_with_content(
            grab_offset,
            logical_size,
            panels,
            None,
            Some(column),
            None,
            cx,
        )
    }

    /// Opens DragHost with all active cached PanelViews belonging to one Main DockArea.
    pub fn open_dock_area_at_cursor(
        &mut self,
        side: DockSide,
        panels: Vec<PanelId>,
        grab_offset: Point,
        logical_size: Size,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        let can_transfer = self.controller.update(cx, |controller, _| {
            controller.can_begin_dock_area_view_transfer(side, &panels)
        });
        if !can_transfer {
            return Err(panels.first().copied().map_or(
                DragHostError::ContentTransferUnavailable,
                DragHostError::PanelTransferUnavailable,
            ));
        }
        self.open_at_cursor_with_content(
            grab_offset,
            logical_size,
            panels,
            None,
            None,
            Some(side),
            cx,
        )
    }

    fn open_at_cursor_with_content(
        &mut self,
        grab_offset: Point,
        logical_size: Size,
        panels: Vec<PanelId>,
        group: Option<NodeId>,
        column: Option<NodeId>,
        dock_area: Option<DockSide>,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        if self.active.is_some() {
            return Err(DragHostError::AlreadyActive);
        }
        let owner = self
            .main_native_window
            .ok_or(DragHostError::MainWindowUnavailable)?;
        grab_offset
            .validate()
            .map_err(|_| DragHostError::InvalidGrabOffset)?;
        logical_size
            .validate_positive()
            .map_err(|_| DragHostError::InvalidLogicalSize)?;

        let id = DragHostId::from_raw(self.next_id);
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(DragHostError::IdExhausted)?;
        self.next_id = next_id;

        let cursor = self.native_ops.cursor_position()?;
        let owner_scale = valid_scale(self.native_ops.scale_factor(owner)?)
            .ok_or(DragHostError::InvalidScaleFactor)?;
        let physical_grab_offset =
            DockGeometry::physical_offset_from_logical(grab_offset, owner_scale)
                .ok_or(DragHostError::InvalidGrabOffset)?;
        let physical_origin =
            DockGeometry::physical_position_for_grab(cursor, physical_grab_offset);
        let logical_origin = DockGeometry::logical_point_from_core_physical(
            Point::new(physical_origin.x as f32, physical_origin.y as f32),
            owner_scale,
        )
        .ok_or(DragHostError::InvalidScreenPosition)?;

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(logical_origin.x), px(logical_origin.y)),
                size(px(logical_size.width), px(logical_size.height)),
            ))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::Floating,
            is_movable: false,
            app_owns_titlebar_drag: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        };
        let theme = self.theme;
        let controller = self.controller.clone();
        let surface_panel = panels.first().copied();
        let window = cx
            .open_window(options, move |_, app| {
                app.new(|_| DragHostSurface {
                    theme,
                    controller,
                    controller_observed: false,
                    panel: surface_panel,
                    group,
                    column,
                    dock_area,
                    handoff_placeholder: false,
                })
            })
            .map_err(|error| DragHostError::OpenWindow(error.to_string()))?;

        let window_id = match window.update(cx, |_, window, _| {
            window.window_handle().window_id()
        }) {
            Ok(window_id) => window_id,
            Err(error) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return Err(DragHostError::WindowUpdate(error.to_string()));
            }
        };
        let native_window = match window.update(cx, |_, window, _| {
            self.native_ops.hwnd_of(window)
        }) {
            Ok(Ok(native_window)) => native_window,
            Ok(Err(error)) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return Err(DragHostError::Native(error));
            }
            Err(error) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return Err(DragHostError::WindowUpdate(error.to_string()));
            }
        };

        if let Err(error) = self.native_ops.set_owner(native_window, owner) {
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return Err(DragHostError::Native(error));
        }
        if !platform::hide_from_taskbar(native_window) {
            tracing::warn!("could not hide the new DragHost from the taskbar");
        }
        // The destination monitor may use a different DPI from MainHost. Read
        // the new HWND's scale and recompute the grab offset before its first show.
        let mut destination_scale = match self.native_ops.scale_factor(native_window) {
            Ok(scale) => match valid_scale(scale) {
                Some(scale) => scale,
                None => {
                    self.discard_unregistered_window(&window, native_window, cx);
                    return Err(DragHostError::InvalidScaleFactor);
                }
            },
            Err(error) => {
                self.discard_unregistered_window(&window, native_window, cx);
                return Err(DragHostError::Native(error));
            }
        };
        let Some(destination_grab_offset) =
            DockGeometry::physical_offset_from_logical(grab_offset, destination_scale)
        else {
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::InvalidGrabOffset);
        };
        let destination_origin =
            DockGeometry::physical_position_for_grab(cursor, destination_grab_offset);
        if let Err(error) = self.native_ops.move_window(native_window, destination_origin) {
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::Native(error));
        }

        // Moving to the destination can itself cross a DPI boundary. Re-read
        // scale and correct both the grab point and requested client size.
        let final_scale = match self.native_ops.scale_factor(native_window) {
            Ok(scale) => match valid_scale(scale) {
                Some(scale) => scale,
                None => {
                    self.discard_unregistered_window(&window, native_window, cx);
                    return Err(DragHostError::InvalidScaleFactor);
                }
            },
            Err(error) => {
                self.discard_unregistered_window(&window, native_window, cx);
                return Err(DragHostError::Native(error));
            }
        };
        if (final_scale - destination_scale).abs() > 0.001 {
            destination_scale = final_scale;
            let Some(final_grab_offset) =
                DockGeometry::physical_offset_from_logical(grab_offset, destination_scale)
            else {
                self.discard_unregistered_window(&window, native_window, cx);
                return Err(DragHostError::InvalidGrabOffset);
            };
            let final_origin =
                DockGeometry::physical_position_for_grab(cursor, final_grab_offset);
            if final_origin != destination_origin {
                if let Err(error) = self.native_ops.move_window(native_window, final_origin) {
                    self.discard_unregistered_window(&window, native_window, cx);
                    return Err(DragHostError::Native(error));
                }
            }
        }

        let Some(physical_size) =
            DockGeometry::physical_size_from_logical(logical_size, destination_scale)
        else {
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::InvalidLogicalSize);
        };
        if let Err(error) = self.native_ops.resize_window(native_window, physical_size) {
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::Native(error));
        }

        let host_state_ready = self.controller.update(cx, |controller, controller_cx| {
            let attached = controller.drag_mut().attach_transport_host(id);
            let transferred = match (group, column, dock_area, panels.as_slice()) {
                (Some(group), None, None, [panel]) => {
                    controller.begin_group_view_transfer(group, *panel)
                }
                (None, Some(column), None, panels) => {
                    controller.begin_column_view_transfer(column, panels)
                }
                (None, None, Some(side), panels) => {
                    controller.begin_dock_area_view_transfer(side, panels)
                }
                (None, None, None, [panel]) => controller.begin_panel_view_transfer(*panel),
                (None, None, None, []) => true,
                _ => false,
            };
            if transferred {
                controller_cx.notify();
            }
            attached && transferred
        });
        if !host_state_ready {
            self.controller.update(cx, |controller, _| {
                controller.drag_mut().clear_transport_host(id);
            });
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(panels.first().copied().map_or(
                DragHostError::ContentTransferUnavailable,
                DragHostError::PanelTransferUnavailable,
            ));
        }

        let visible = panels.is_empty();
        if visible
            && let Err(error) = self.native_ops.show_no_activate(native_window)
        {
            self.controller.update(cx, |controller, _| {
                controller.drag_mut().clear_transport_host(id);
            });
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::Native(error));
        }

        self.active = Some(ActiveDragHost {
            id,
            window,
            window_id,
            native_window,
            panels: panels.clone(),
            grab_offset,
            visible,
            drop_target: None,
            target_role: None,
            free_area_bounds: None,
            pending_release_cursor: None,
            focus_loss_since: None,
            source_detach_subscription: None,
            handoff: None,
        });

        if !panels.is_empty() {
            let controller = self.controller.clone();
            let subscription = cx.observe(&controller, move |manager, controller, manager_cx| {
                // An observer queued by a previous drag must never affect a newer
                // DragHost session. Its Subscription is also dropped by close().
                if manager.active_drag_host_id() != Some(id) {
                    return;
                }

                let source_detached = controller.update(manager_cx, |controller, _| {
                    controller.drag().host_session().is_some_and(|session| {
                        session.transport_host() == Some(id)
                            && !session.transferred_panels().is_empty()
                            && session.all_panel_views_source_detached()
                    })
                });
                if !source_detached {
                    return;
                }
                if let Err(error) = manager.show_pending_drag_host(id) {
                    tracing::warn!(
                        ?id,
                        %error,
                        "could not show DragHost after source placeholder was rendered"
                    );
                    if let Err(close_error) = manager.close(id, manager_cx) {
                        tracing::warn!(
                            ?id,
                            %close_error,
                            "could not close DragHost after its delayed show failed"
                        );
                    }
                    manager.controller.update(manager_cx, |controller, controller_cx| {
                        if let Err(cancel_error) = controller.cancel_pointer_interaction() {
                            tracing::debug!(
                                ?id,
                                %cancel_error,
                                "could not cancel Panel drag after DragHost show failure"
                            );
                        }
                        controller_cx.notify();
                    });
                }
            });
            if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
                active.source_detach_subscription = Some(subscription);
            }
        }

        self.schedule_drop_tracking(cx, id);
        Ok(id)
    }

    /// Moves the active DragHost so the same logical grab point stays under
    /// the current cursor. The window never activates during movement.
    pub fn move_with_cursor(
        &mut self,
        id: DragHostId,
        grab_offset: Point,
    ) -> Result<(), DragHostError> {
        let active = self.active.as_ref().ok_or(DragHostError::NotActive(id))?;
        if active.id != id {
            return Err(DragHostError::NotActive(id));
        }
        grab_offset
            .validate()
            .map_err(|_| DragHostError::InvalidGrabOffset)?;
        let cursor = self.native_ops.cursor_position()?;
        self.move_to_cursor(id, grab_offset, cursor)
    }

    fn move_to_cursor(
        &self,
        id: DragHostId,
        grab_offset: Point,
        cursor: platform::PhysicalPoint,
    ) -> Result<(), DragHostError> {
        let active = self.active.as_ref().ok_or(DragHostError::NotActive(id))?;
        if active.id != id {
            return Err(DragHostError::NotActive(id));
        }
        let scale = valid_scale(self.native_ops.scale_factor(active.native_window)?)
            .ok_or(DragHostError::InvalidScaleFactor)?;
        let physical_grab_offset =
            DockGeometry::physical_offset_from_logical(grab_offset, scale)
                .ok_or(DragHostError::InvalidGrabOffset)?;
        let position = DockGeometry::physical_position_for_grab(cursor, physical_grab_offset);
        self.native_ops.move_window(active.native_window, position)?;
        Ok(())
    }

    fn schedule_drop_tracking(&self, cx: &mut Context<Self>, id: DragHostId) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let Ok(keep_tracking) = manager.update(app, |manager, manager_cx| {
                    manager.tick_drop_tracking(id, manager_cx)
                }) else {
                    break;
                };
                if !keep_tracking {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_drop_tracking(&mut self, id: DragHostId, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.active.as_ref().filter(|active| active.id == id) else {
            return false;
        };
        if self.native_ops.key_down(NativeKey::Escape).unwrap_or(false) {
            self.cancel_active_drag(id, cx);
            return false;
        }
        let native_drag_host = active.native_window;
        let grab_offset = active.grab_offset;
        let floating_manager = self.floating_manager.clone();

        // DragHost is shown without activation, so Windows may briefly report
        // no foreground window while focus moves between managed dock hosts.
        // Debounce that transition; a sustained loss still cancels the gesture.
        let foreground = self.native_ops.foreground_window().ok().flatten();
        let managed_foreground = foreground.is_some_and(|window| {
            window == native_drag_host
                || floating_manager.as_ref().is_some_and(|manager| {
                    manager.update(cx, |manager, _| {
                        manager.is_managed_native_window(window)
                    })
                })
        });
        let now = Instant::now();
        let button_is_down = self.native_ops.left_button_down().unwrap_or(true);
        let focus_loss_expired = if let Some(active) =
            self.active.as_mut().filter(|active| active.id == id)
        {
            if managed_foreground {
                active.focus_loss_since = None;
                false
            } else if !button_is_down {
                // A released gesture must still finish at its captured pointer
                // position, even if Windows has already changed foreground.
                false
            } else {
                let since = *active.focus_loss_since.get_or_insert(now);
                now.duration_since(since) >= Duration::from_millis(150)
            }
        } else {
            true
        };
        if focus_loss_expired {
            tracing::debug!(
                ?id,
                ?foreground,
                "foreground stayed outside managed docking windows; cancelling DragHost"
            );
            self.cancel_active_drag_on_focus_loss(id, cx);
            return false;
        }

        let tracking_state = self
            .controller
            .update(cx, |controller, _| {
                let Some(host_session) = controller.drag().host_session() else {
                    return None;
                };
                if host_session.transport_host() != Some(id) {
                    return None;
                }
                if host_session.transferred_panels().iter().any(|panel| {
                    !host_session.panel_view_source_detached(*panel)
                })
                {
                    return Some(None);
                }
                let DragState::Dragging { payload, .. } = controller.drag().session().state() else {
                    return None;
                };
                Some(Some((
                    payload,
                    host_session.handle(),
                    host_session.source_host(),
                    host_session.source_window_id(),
                )))
            });
        let Some(tracking_state) = tracking_state else {
            if let Err(error) = self.close(id, cx) {
                tracing::warn!(?id, %error, "could not close DragHost after its source gesture ended");
            }
            return false;
        };
        let Some((payload, handle, source_host, source_window_id)) = tracking_state else {
            // The source still owns the AnyView. Remember an early release, but
            // keep DragHost alive until the source has rendered its placeholder.
            if !self.native_ops.left_button_down().unwrap_or(true) {
                let release_cursor = match self.native_ops.cursor_position() {
                    Ok(cursor) => cursor,
                    Err(error) => {
                        tracing::warn!(
                            ?id,
                            %error,
                            "could not capture cursor position after early mouse release"
                        );
                        self.cancel_active_drag(id, cx);
                        return false;
                    }
                };
                if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
                    active.pending_release_cursor.get_or_insert(release_cursor);
                }
            }
            return true;
        };

        // If the button was released while the source still owned the AnyView,
        // finish at the captured release point rather than a later cursor point.
        let pending_release_cursor = self
            .active
            .as_ref()
            .filter(|active| active.id == id)
            .and_then(|active| active.pending_release_cursor);
        let cursor = match pending_release_cursor {
            Some(cursor) => cursor,
            None => match self.native_ops.cursor_position() {
                Ok(cursor) => cursor,
                Err(error) => {
                    tracing::warn!(?id, %error, "could not read cursor position during DragHost tracking");
                    self.cancel_active_drag(id, cx);
                    return false;
                }
            },
        };
        if let Err(error) = self.move_to_cursor(id, grab_offset, cursor) {
            tracing::warn!(?id, %error, "DragHost could not follow the cursor");
            self.cancel_active_drag(id, cx);
            return false;
        }

        let Some(floating_manager) = floating_manager else {
            self.cancel_active_drag(id, cx);
            return false;
        };
        let excluded_source_window = matches!(
            handle,
            DragHandle::FloatingTitle | DragHandle::FloatingCapture
        )
        .then_some(source_window_id)
        .flatten();
        let target_window = floating_manager
            .update(cx, |manager, _| {
                manager.docking_window_at_point(
                    cursor,
                    native_drag_host,
                    excluded_source_window,
                )
            });
        let source_point = floating_manager.update(cx, |manager, _| {
            manager.core_point_for_drag_source(source_host, source_window_id, cursor)
        });
        let Some(source_point) = source_point else {
            tracing::warn!(
                ?id,
                ?source_host,
                ?source_window_id,
                "DragHost source window is no longer available"
            );
            self.cancel_active_drag(id, cx);
            return false;
        };
        let source = crate::input::drag::drop_source(payload);

        let update: Result<(Option<DropTarget>, Option<Rect>), String> =
            self.controller.update(cx, |controller, controller_cx| {
            let target_result: Result<Option<DropTarget>, dock_core::ComputeRectsError> =
                if matches!(payload, DragPayload::DockArea { .. }) {
                    // DockArea has no docking destination resolver. It may detach
                    // over managed hosts or empty desktop space, but a foreign
                    // topmost window must block the drop.
                    if matches!(target_window, Some(DockingWindowAtPoint::ForeignWindow)) {
                        Ok(None)
                    } else {
                        Ok(Some(DropTarget::FreeArea))
                    }
                } else {
                    match (source, target_window) {
                        (Some(source), Some(DockingWindowAtPoint::Main { point, .. })) => {
                            controller
                                .prepare_drop_resolver(source)
                                .and_then(|resolver| resolver.resolve(point))
                        }
                        (
                            Some(source),
                            Some(DockingWindowAtPoint::Floating {
                                group,
                                window_id,
                                point,
                            }),
                        ) => controller
                            .prepare_floating_window_drop_resolver(window_id, group, source)
                            .and_then(|resolver| resolver.resolve(point)),
                        (Some(_), Some(DockingWindowAtPoint::ForeignWindow)) => Ok(None),
                        (Some(source), None) => controller
                            .prepare_drop_resolver(source)
                            .map(|resolver| {
                                resolver
                                    .zones()
                                    .iter()
                                    .any(|zone| {
                                        zone.target == DropTarget::FreeArea
                                            && zone.rect.contains(source_point).unwrap_or(false)
                                    })
                                    .then_some(DropTarget::FreeArea)
                            }),
                        (None, _) => Ok(None),
                    }
                };
            let target = target_result.map_err(|error| error.to_string())?;

            // A resolved dock target supplies coordinates in that target Host.
            // FreeArea has no target Host, so keep the pointer in the original
            // source Host's local coordinate system instead of mixing Main/Floating
            // coordinate spaces in the DragDropRequest.
            let pointer_point = if matches!(payload, DragPayload::DockArea { .. }) {
                // FreeArea bounds are calculated in the original MainHost's
                // coordinate space, never in a window under the release cursor.
                source_point
            } else {
                target_window
                    .and_then(|target| match target {
                        DockingWindowAtPoint::Main { point, .. }
                        | DockingWindowAtPoint::Floating { point, .. } => Some(point),
                        DockingWindowAtPoint::ForeignWindow => None,
                    })
                    .unwrap_or(source_point)
            };
            controller
                .drag_mut()
                .session_mut()
                .pointer_move(pointer_point)
                .map_err(|error| error.to_string())?;
            controller
                .drag_mut()
                .session_mut()
                .set_drop_target(target)
                .map_err(|error| error.to_string())?;

            let local_free_area_bounds = if target == Some(DropTarget::FreeArea) {
                crate::input::drag::floating_bounds_for_drag(
                    controller,
                    payload,
                    source_point,
                    source_host,
                    source_window_id,
                )
            } else {
                None
            };
            controller_cx.notify();
            Ok((target, local_free_area_bounds))
        });
        let (target, local_free_area_bounds) = match update {
            Ok(update) => update,
            Err(error) => {
                tracing::warn!(
                    ?id,
                    %error,
                    "DragHost drop tracking failed; cancelling the active session"
                );
                self.cancel_active_drag(id, cx);
                return false;
            }
        };
        let screen_free_area_bounds = local_free_area_bounds.and_then(|bounds| {
            floating_manager.update(cx, |manager, _| {
                manager.screen_bounds_for_drag_source(source_host, bounds)
            })
        });
        if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
            active.drop_target = target;
            active.target_role = target_window.and_then(DockingWindowAtPoint::role);
            active.free_area_bounds = screen_free_area_bounds;
        }

        match self.native_ops.left_button_down() {
            Ok(true) => true,
            Ok(false) => {
                self.finish_active_drag(id, cx);
                false
            }
            Err(error) => {
                tracing::warn!(?id, %error, "could not read mouse button state for DragHost");
                self.cancel_active_drag(id, cx);
                false
            }
        }
    }

    fn finish_active_drag(&mut self, id: DragHostId, cx: &mut Context<Self>) {
        let Some(active) = self.active.as_ref().filter(|active| active.id == id) else {
            return;
        };
        let target = active.drop_target;
        let free_area_bounds = active.free_area_bounds;
        let panel = active.panels.first().copied();

        let finished = self.controller.update(cx, |controller, controller_cx| {
            let Some(preview) = controller.drag().session().preview() else {
                let _ = controller.cancel_pointer_interaction();
                let _ = controller.drag_mut().session_mut().reset();
                controller.drag_mut().clear_resolver();
                controller.drag_mut().clear_layout_version();
                controller_cx.notify();
                return false;
            };

            let request = match controller
                .drag_mut()
                .session_mut()
                .pointer_up(preview.point, target)
            {
                Ok(request) => request,
                Err(error) => {
                    tracing::debug!(?id, %error, "DragHost release did not produce a drop request");
                    None
                }
            };

            if let Some(mut request) = request {
                if request.target == Some(DropTarget::FreeArea) {
                    if let Some(bounds) = free_area_bounds {
                        request.floating_bounds = Some(bounds);
                    } else {
                        tracing::debug!(?id, "DragHost FreeArea drop has no valid screen bounds");
                    }
                }

                let can_resolve = request.target != Some(DropTarget::FreeArea)
                    || request.floating_bounds.is_some();
                if can_resolve {
                    let action = match request.payload {
                        DragPayload::DockArea { side } => {
                            let command = if request.target == Some(DropTarget::FreeArea) {
                                controller
                                    .layout()
                                    .main_area(side)
                                    .zip(request.floating_bounds)
                                    .map(|(area, bounds)| DockCommand::Float {
                                        node: area.root,
                                        bounds,
                                    })
                            } else {
                                None
                            };
                            Ok(command.map_or(DragDropAction::NoOp, DragDropAction::Command))
                        }
                        _ => {
                            let metrics = controller.metrics();
                            controller.layout().resolve_drag_drop(&request, &metrics)
                        }
                    };
                    match action {
                        Ok(DragDropAction::NoOp) => {}
                        Ok(DragDropAction::Command(command)) => {
                            if let Err(error) = controller.dispatch_in(command, controller_cx) {
                                tracing::warn!(?id, %error, "DragHost drop command was rejected");
                            }
                        }
                        Err(error) => controller.report_drop_resolution_error_in(error, controller_cx),
                    }
                }
            }

            let _ = controller.drag_mut().session_mut().reset();
            controller.drag_mut().clear_resolver();
            controller.drag_mut().clear_layout_version();
            controller_cx.notify();
            true
        });

        if let Some(panel) = panel
            && !finished
        {
            tracing::debug!(?id, ?panel, "DragHost release was cancelled; source view will be restored");
        }

        let floating_target = if finished {
            panel.and_then(|panel| {
                self.controller.update(cx, |controller, _| {
                    floating_group_for_panel(controller.layout(), panel)
                }).ok().flatten()
            })
        } else {
            None
        };

        if let Some(target_group) = floating_target {
            if !self.begin_floating_handoff(id, target_group, cx) {
                tracing::warn!(
                    ?id,
                    ?target_group,
                    "could not prepare the DragHost-to-FloatingHost handoff; closing DragHost"
                );
                if let Err(error) = self.close(id, cx) {
                    tracing::warn!(?id, %error, "could not close DragHost after handoff setup failed");
                }
            }
            return;
        }

        if let Err(error) = self.close(id, cx) {
            tracing::warn!(?id, %error, "could not close DragHost after release");
        }
    }

    fn begin_floating_handoff(
        &mut self,
        id: DragHostId,
        target_group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return false;
        };
        active.handoff = Some(DragHostHandoff {
            target_group,
            source_placeholder_rendered: false,
            source_released: false,
        });
        let window = active.window.clone();

        let manager = cx.entity().downgrade();
        match window.update(cx, move |surface, window, host_cx| {
            // Stop mounting the transferred AnyView, but keep the native
            // DragHost surface alive at the cursor's final position.
            surface.handoff_placeholder = true;
            host_cx.notify();
            window.on_next_frame(move |_, app| {
                let _ = manager.update(app, |manager, manager_cx| {
                    manager.source_placeholder_rendered(id, manager_cx);
                });
            });
        }) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(?id, %error, "could not switch DragHost to an empty handoff placeholder");
                if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
                    active.handoff = None;
                }
                false
            }
        }
    }

    fn source_placeholder_rendered(&mut self, id: DragHostId, cx: &mut Context<Self>) {
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return;
        };
        let Some(handoff) = active.handoff.as_mut() else {
            return;
        };
        handoff.source_placeholder_rendered = true;

        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let Ok(keep_tracking) = manager.update(app, |manager, manager_cx| {
                    manager.try_release_handoff_source(id, manager_cx)
                }) else {
                    break;
                };
                if !keep_tracking {
                    break;
                }
            }
        })
        .detach();
    }

    /// Releases the cached views only after the empty DragHost frame has rendered.
    /// The persistent FloatingHost is shown in the same callback before the next
    /// application frame, so it can mount the views without an empty visible frame.
    fn try_release_handoff_source(
        &mut self,
        id: DragHostId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((target_group, panels, placeholder_rendered, source_released)) = self
            .active
            .as_ref()
            .filter(|active| active.id == id)
            .and_then(|active| {
                let handoff = active.handoff?;
                Some((
                    handoff.target_group,
                    active.panels.clone(),
                    handoff.source_placeholder_rendered,
                    handoff.source_released,
                ))
            })
        else {
            return false;
        };
        if !placeholder_rendered {
            return false;
        }

        if !source_released {
            let released = self.controller.update(cx, |controller, controller_cx| {
                for panel in &panels {
                    controller.finish_panel_view_transfer(*panel);
                }
                controller.drag_mut().finish_host_session();
                controller_cx.notify();
            });
            if released.is_err() {
                return true;
            }
            if let Some(active) = self.active.as_mut().filter(|active| active.id == id)
                && let Some(handoff) = active.handoff.as_mut()
            {
                handoff.source_released = true;
            }
        }

        let target_shown = self
            .floating_manager
            .as_ref()
            .and_then(|manager| {
                manager
                    .update(cx, |manager, manager_cx| {
                        manager.show_handoff_target(target_group, manager_cx)
                    })
                    .ok()
            })
            .unwrap_or(false);
        !target_shown
    }

    pub(crate) fn pending_handoff_panels(
        &self,
        target_group: FloatingGroupId,
    ) -> Option<Vec<PanelId>> {
        let active = self.active.as_ref()?;
        let handoff = active.handoff?;
        (handoff.target_group == target_group && handoff.source_released)
            .then(|| active.panels.clone())
    }

    pub(crate) fn complete_floating_handoff(
        &mut self,
        target_group: FloatingGroupId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(handoff) = active.handoff else {
            return;
        };
        if handoff.target_group != target_group
            || !handoff.source_released
            || !active.panels.iter().all(|panel| rendered_panels.contains(panel))
        {
            return;
        }

        let id = active.id;
        if let Err(error) = self.close(id, cx) {
            tracing::warn!(
                ?id,
                ?target_group,
                %error,
                "could not close DragHost after FloatingHost rendered transferred content"
            );
        }
    }

    fn cancel_active_drag(&mut self, id: DragHostId, cx: &mut Context<Self>) {
        if let Err(error) = self.close(id, cx) {
            tracing::warn!(?id, %error, "could not close DragHost after cancellation");
        }
        self.controller.update(cx, |controller, controller_cx| {
            if let Err(error) = controller.cancel_pointer_interaction_on_escape() {
                tracing::debug!(?id, %error, "could not cancel DragHost pointer interaction");
            }
            if let Err(error) = controller.drag_mut().session_mut().reset() {
                tracing::debug!(?id, %error, "could not reset cancelled DragHost session");
            }
            controller.drag_mut().clear_resolver();
            controller.drag_mut().clear_layout_version();
            controller_cx.notify();
        });
    }

    fn cancel_active_drag_on_focus_loss(&mut self, id: DragHostId, cx: &mut Context<Self>) {
        if let Err(error) = self.close(id, cx) {
            tracing::warn!(
                ?id,
                %error,
                "could not close DragHost after application focus loss"
            );
        }
        self.controller.update(cx, |controller, controller_cx| {
            if let Err(error) = controller.cancel_pointer_interaction_on_focus_loss() {
                tracing::debug!(
                    ?id,
                    %error,
                    "could not cancel DragHost pointer interaction after focus loss"
                );
            }
            if let Err(error) = controller.drag_mut().session_mut().reset() {
                tracing::debug!(?id, %error, "could not reset DragHost session after focus loss");
            }
            controller.drag_mut().clear_resolver();
            controller.drag_mut().clear_layout_version();
            controller_cx.notify();
        });
    }

    /// Cancels the active content drag and restores its cached PanelView.
    pub(crate) fn cancel_active(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.active.as_ref().map(|active| active.id) else {
            return false;
        };
        self.cancel_active_drag(id, cx);
        true
    }

    fn show_pending_drag_host(&mut self, id: DragHostId) -> Result<(), DragHostError> {
        let active = self.active.as_mut().ok_or(DragHostError::NotActive(id))?;
        if active.id != id {
            return Err(DragHostError::NotActive(id));
        }
        if active.visible {
            return Ok(());
        }

        self.native_ops.show_no_activate(active.native_window)?;
        active.visible = true;
        Ok(())
    }

    /// Closes the matching DragHost. Repeated close calls are harmless.
    pub fn close(
        &mut self,
        id: DragHostId,
        cx: &mut Context<Self>,
    ) -> Result<(), DragHostError> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        if active.id != id {
            self.active = Some(active);
            return Err(DragHostError::NotActive(id));
        }
        drop(active.source_detach_subscription.take());

        if let Err(error) = self.native_ops.hide(active.native_window) {
            tracing::debug!(
                ?id,
                %error,
                "DragHost could not be confirmed hidden during close"
            );
        }

        // GPUI may already have disposed the window (for example after a native
        // close). In either case the manager must release its runtime entry;
        // retaining a failed WindowHandle update here would leave a dead
        // WindowId/HWND and PanelView transfer registered indefinitely.
        let window_update_error = active
            .window
            .update(cx, |_, window, _| window.remove_window())
            .err();
        if let Some(error) = &window_update_error {
            tracing::debug!(
                ?id,
                %error,
                "DragHost window was already unavailable during close"
            );
        }

        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_transport_host(id);
            for panel in active.panels {
                controller.finish_panel_view_transfer(panel);
            }
            // The temporary surface is no longer a valid presentation host.
            // Only now may the cached PanelViews return to the persistent Host.
            controller.drag_mut().finish_host_session();
            controller_cx.notify();
        });

        match window_update_error {
            Some(error) => Err(DragHostError::WindowUpdate(error.to_string())),
            None => Ok(()),
        }
    }

    /// Returns the Dragging role for the current temporary window, if present.
    pub fn role_for_window_id(&self, window_id: WindowId) -> Option<DockWindowRole> {
        let active = self.active.as_ref()?;
        (active.window_id == window_id).then_some(DockWindowRole::Dragging { id: active.id })
    }

    /// Returns the active DragHost HWND for drop-target exclusion and focus handoff.
    ///
    /// The drag/drop router passes this value as `exclude` to
    /// `NativeWindowOps::window_from_point`. Focus-loss handling also uses it
    /// to distinguish a transfer into DragHost from a gesture leaving the app.
    pub(crate) fn active_native_window_id(&self) -> Option<NativeWindowId> {
        self.active.as_ref().map(|active| active.native_window)
    }

    /// Returns the identifier of the currently active DragHost, if present.
    pub fn active_drag_host_id(&self) -> Option<DragHostId> {
        self.active.as_ref().map(|active| active.id)
    }

    /// Returns the selected Core target for the active DragHost gesture.
    pub fn active_drop_target(&self) -> Option<DropTarget> {
        self.active.as_ref()?.drop_target
    }

    /// Returns the persistent host currently under the cursor, when registered.
    pub fn active_target_role(&self) -> Option<DockWindowRole> {
        self.active.as_ref()?.target_role
    }

    /// Returns the proposed persistent Floating bounds in virtual-desktop pixels.
    pub fn active_free_area_bounds(&self) -> Option<Rect> {
        self.active.as_ref()?.free_area_bounds
    }

    fn discard_unregistered_window(
        &self,
        window: &WindowHandle<DragHostSurface>,
        native_window: NativeWindowId,
        cx: &mut Context<Self>,
    ) {
        let _ = self.native_ops.hide(native_window);
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DragHostError {
    #[error("a DragHost is already active")]
    AlreadyActive,
    #[error("the main native window has not been attached")]
    MainWindowUnavailable,
    #[error("the DragHost size must be positive")]
    InvalidLogicalSize,
    #[error("the pointer grab offset is invalid")]
    InvalidGrabOffset,
    #[error("the native monitor scale factor is invalid")]
    InvalidScaleFactor,
    #[error("the pointer position cannot be converted to GPUI screen coordinates")]
    InvalidScreenPosition,
    #[error("no active DragHost matches {0:?}")]
    NotActive(DragHostId),
    #[error("Panel {0:?} has no cached view or matching active content drag session")]
    PanelTransferUnavailable(PanelId),
    #[error("the dragged composition has no matching active content drag session")]
    ContentTransferUnavailable,
    #[error("DragHost runtime identifiers are exhausted")]
    IdExhausted,
    #[error("GPUI could not create a DragHost window: {0}")]
    OpenWindow(String),
    #[error("GPUI window operation failed: {0}")]
    WindowUpdate(String),
    #[error(transparent)]
    Native(#[from] NativeError),
}

struct DragHostSurface {
    theme: DockTheme,
    controller: Entity<DockController>,
    controller_observed: bool,
    panel: Option<PanelId>,
    group: Option<NodeId>,
    column: Option<NodeId>,
    dock_area: Option<DockSide>,
    handoff_placeholder: bool,
}

struct DragGroupPresentation {
    group: GroupRect,
    strip: TabStripRect,
    headers: Vec<TabHeaderRect>,
    overflow: Option<TabOverflowRect>,
    presentation: GroupPresentation,
    column: Option<NodeId>,
    panel: Option<(PanelRect, AnyView)>,
    metrics: DockMetrics,
}

struct DragColumnPresentation {
    column: ColumnRect,
    groups: Vec<GroupRect>,
    panels: Vec<(PanelRect, Option<AnyView>)>,
    tab_strips: Vec<TabStripRect>,
    tab_headers: Vec<TabHeaderRect>,
    tab_overflows: Vec<TabOverflowRect>,
    presentations: std::collections::HashMap<NodeId, GroupPresentation>,
    group_ids: Vec<NodeId>,
    mode: ColumnMode,
    rail_items: Vec<ColumnDisplayItem>,
    splitters: Vec<SplitterRect>,
    vertical_overflow: Vec<VerticalOverflow>,
    metrics: DockMetrics,
}

struct DragDockAreaPresentation {
    columns: Vec<ColumnRect>,
    groups: Vec<GroupRect>,
    panels: Vec<(PanelRect, Option<AnyView>)>,
    tab_strips: Vec<TabStripRect>,
    tab_headers: Vec<TabHeaderRect>,
    tab_overflows: Vec<TabOverflowRect>,
    presentations: HashMap<NodeId, GroupPresentation>,
    rail_display_orders: HashMap<NodeId, Vec<ColumnDisplayItem>>,
    node_ids: HashSet<NodeId>,
    node_sides: HashMap<NodeId, DockSide>,
    column_sides: HashMap<NodeId, DockSide>,
    column_groups: HashMap<NodeId, Vec<NodeId>>,
    column_modes: HashMap<NodeId, ColumnMode>,
    splitters: Vec<SplitterRect>,
    vertical_overflow: Vec<VerticalOverflow>,
    metrics: DockMetrics,
}

fn dock_area_bounds(columns: &[ColumnRect]) -> Option<Rect> {
    let first = columns.first()?.rect;
    let mut left = first.origin.x;
    let mut top = first.origin.y;
    let mut right = first.origin.x + first.size.width;
    let mut bottom = first.origin.y + first.size.height;

    for column in columns.iter().skip(1) {
        left = left.min(column.rect.origin.x);
        top = top.min(column.rect.origin.y);
        right = right.max(column.rect.origin.x + column.rect.size.width);
        bottom = bottom.max(column.rect.origin.y + column.rect.size.height);
    }

    Some(Rect::new(
        Point::new(left, top),
        Size::new(right - left, bottom - top),
    ))
}

fn translate_rect(rect: Rect, origin: Point) -> Rect {
    Rect::new(
        Point::new(rect.origin.x - origin.x, rect.origin.y - origin.y),
        rect.size,
    )
}

impl Render for DragHostSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.controller_observed {
            let controller = self.controller.clone();
            cx.observe(&controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.window_background));

        if self.handoff_placeholder {
            return root;
        }

        if let Some(side) = self.dock_area
            && let Some(snapshot) = self.controller.update(cx, |controller, _| {
                let (root, panels) = {
                    let session = controller.drag().host_session()?;
                    if session.source_host() != DragSourceHost::Main
                        || !matches!(
                            session.payload(),
                            DragPayload::DockArea { side: active } if active == side
                        )
                        || !session.all_panel_views_source_detached()
                    {
                        return None;
                    }
                    (
                        controller.layout().main_area(side)?.root,
                        session.transferred_panels().to_vec(),
                    )
                };

                let mut node_ids = HashSet::new();
                let mut column_ids = HashSet::new();
                let mut column_groups = HashMap::new();
                crate::dock_host::collect_subtree_nodes(
                    controller.layout(),
                    root,
                    &mut node_ids,
                    &mut column_ids,
                    &mut column_groups,
                );

                let rects = controller.geometry().ok()?.clone();
                let source_columns = rects
                    .columns
                    .iter()
                    .filter(|column| column_ids.contains(&column.column))
                    .copied()
                    .collect::<Vec<_>>();
                let area_rect = dock_area_bounds(&source_columns)?;
                let columns = source_columns
                    .iter()
                    .map(|column| {
                        ColumnRect::new(
                            column.column,
                            translate_rect(column.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let group_ids = column_groups
                    .values()
                    .flat_map(|groups| groups.iter().copied())
                    .collect::<HashSet<_>>();
                let groups = rects
                    .groups
                    .iter()
                    .filter(|group| group_ids.contains(&group.group))
                    .map(|group| {
                        GroupRect::new(
                            group.group,
                            translate_rect(group.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_strips = rects
                    .tab_strips
                    .iter()
                    .filter(|strip| group_ids.contains(&strip.group))
                    .map(|strip| {
                        TabStripRect::new(
                            strip.group,
                            translate_rect(strip.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_headers = rects
                    .tab_headers
                    .iter()
                    .filter(|header| group_ids.contains(&header.group))
                    .map(|header| {
                        TabHeaderRect::new(
                            header.group,
                            header.panel,
                            header.index,
                            translate_rect(header.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_overflows = rects
                    .tab_overflows
                    .iter()
                    .filter(|overflow| group_ids.contains(&overflow.group))
                    .map(|overflow| {
                        TabOverflowRect::new(
                            overflow.group,
                            translate_rect(overflow.button_rect, area_rect.origin),
                            overflow.hidden_tabs.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let presentations = collect_group_presentations(
                    controller.layout(),
                    controller.registry(),
                    &node_ids,
                    Some(&rects),
                );
                let panel_views = rects
                    .panels
                    .iter()
                    .filter(|panel| panels.contains(&panel.panel))
                    .filter_map(|panel| {
                        let view = controller.panel_view_for_drag(panel.panel)?;
                        Some((
                            PanelRect::new(
                                panel.panel,
                                translate_rect(panel.rect, area_rect.origin),
                            ),
                            Some(view),
                        ))
                    })
                    .collect::<Vec<_>>();
                let column_modes = column_ids
                    .iter()
                    .filter_map(|column| match controller.layout().node(*column) {
                        Some(Node::Column(value)) => Some((*column, value.mode)),
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>();
                let rail_display_orders = column_modes
                    .iter()
                    .filter(|(_, mode)| mode.is_rail())
                    .filter_map(|(column, _)| {
                        controller
                            .layout()
                            .column_display_order(*column)
                            .ok()
                            .map(|order| (*column, order))
                    })
                    .collect::<HashMap<_, _>>();
                let splitters = rects
                    .splitters
                    .iter()
                    .filter(|splitter| node_ids.contains(&splitter.owner))
                    .map(|splitter| {
                        SplitterRect::new(
                            splitter.owner,
                            splitter.boundary_index,
                            splitter.axis,
                            translate_rect(splitter.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let vertical_overflow = rects
                    .vertical_overflow
                    .iter()
                    .filter(|overflow| column_ids.contains(&overflow.owner))
                    .map(|overflow| {
                        VerticalOverflow::new(
                            overflow.owner,
                            translate_rect(overflow.viewport, area_rect.origin),
                            translate_rect(overflow.content_extent, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let node_sides = node_ids
                    .iter()
                    .map(|node| (*node, side))
                    .collect::<HashMap<_, _>>();
                let column_sides = column_ids
                    .iter()
                    .map(|column| (*column, side))
                    .collect::<HashMap<_, _>>();

                Some(DragDockAreaPresentation {
                    columns,
                    groups,
                    panels: panel_views,
                    tab_strips,
                    tab_headers,
                    tab_overflows,
                    presentations,
                    rail_display_orders,
                    node_ids,
                    node_sides,
                    column_sides,
                    column_groups,
                    column_modes,
                    splitters,
                    vertical_overflow,
                    metrics: controller.metrics(),
                })
            })
        {
            let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
            root = root.child(MainDockView::render(
                &snapshot.columns,
                &snapshot.groups,
                snapshot.panels,
                None,
                None,
                None,
                &snapshot.tab_strips,
                &snapshot.tab_headers,
                &snapshot.tab_overflows,
                &snapshot.presentations,
                &snapshot.rail_display_orders,
                None,
                None,
                &[],
                snapshot.metrics,
                None,
                toggle_menu,
                true,
                &snapshot.splitters,
                &snapshot.vertical_overflow,
                &snapshot.node_ids,
                &snapshot.node_sides,
                &snapshot.column_sides,
                &snapshot.column_groups,
                &snapshot.column_modes,
                &HashMap::new(),
                self.controller.clone(),
                self.theme,
                true,
            ));
        } else if let Some(column) = self.column
            && let Some(snapshot) = self.controller.update(cx, |controller, _| {
                let (source_host, source_window_id, panels) = {
                    let session = controller.drag().host_session()?;
                    if !matches!(
                        session.payload(),
                        DragPayload::Column { column: active } if active == column
                    ) || !session.all_panel_views_source_detached()
                    {
                        return None;
                    }
                    (
                        session.source_host(),
                        session.source_window_id()?,
                        session.transferred_panels().to_vec(),
                    )
                };
                let rects = match source_host {
                    DragSourceHost::Main => controller.geometry().ok()?.clone(),
                    DragSourceHost::Floating { group } => controller
                        .floating_geometry(source_window_id, group)
                        .ok()?
                        .clone(),
                };
                let source_column = rects
                    .columns
                    .iter()
                    .find(|rect| rect.column == column)
                    .copied()?;
                let Some(Node::Column(column_model)) = controller.layout().node(column) else {
                    return None;
                };
                let group_ids = column_model.groups.clone();
                let group_id_set = group_ids.iter().copied().collect::<HashSet<_>>();
                let group_rects = rects
                    .groups
                    .iter()
                    .filter(|rect| group_id_set.contains(&rect.group))
                    .map(|rect| {
                        GroupRect::new(
                            rect.group,
                            translate_rect(rect.rect, source_column.rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let strip_rects = rects
                    .tab_strips
                    .iter()
                    .filter(|rect| group_id_set.contains(&rect.group))
                    .map(|rect| {
                        TabStripRect::new(
                            rect.group,
                            translate_rect(rect.rect, source_column.rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let header_rects = rects
                    .tab_headers
                    .iter()
                    .filter(|rect| group_id_set.contains(&rect.group))
                    .map(|rect| {
                        TabHeaderRect::new(
                            rect.group,
                            rect.panel,
                            rect.index,
                            translate_rect(rect.rect, source_column.rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let overflow_rects = rects
                    .tab_overflows
                    .iter()
                    .filter(|rect| group_id_set.contains(&rect.group))
                    .map(|rect| {
                        TabOverflowRect::new(
                            rect.group,
                            translate_rect(rect.button_rect, source_column.rect.origin),
                            rect.hidden_tabs.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let presentations = collect_group_presentations(
                    controller.layout(),
                    controller.registry(),
                    &group_id_set,
                    Some(&rects),
                );
                let panel_rects = rects
                    .panels
                    .iter()
                    .filter(|rect| panels.contains(&rect.panel))
                    .filter_map(|rect| {
                        let view = controller.panel_view_for_drag(rect.panel)?;
                        Some((
                            PanelRect::new(
                                rect.panel,
                                translate_rect(rect.rect, source_column.rect.origin),
                            ),
                            Some(view),
                        ))
                    })
                    .collect::<Vec<_>>();
                let local_column = ColumnRect::new(
                    column,
                    translate_rect(source_column.rect, source_column.rect.origin),
                );
                let splitters = rects
                    .splitters
                    .iter()
                    .filter(|splitter| {
                        splitter.owner == column && splitter.axis == dock_core::Axis::Y
                    })
                    .map(|splitter| {
                        SplitterRect::new(
                            splitter.owner,
                            splitter.boundary_index,
                            splitter.axis,
                            translate_rect(splitter.rect, source_column.rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let vertical_overflow = rects.vertical_overflow.iter()
                    .filter(|overflow| overflow.owner == column)
                    .map(|overflow| VerticalOverflow::new(
                        overflow.owner,
                        translate_rect(overflow.viewport, source_column.rect.origin),
                        translate_rect(overflow.content_extent, source_column.rect.origin),
                    ))
                    .collect::<Vec<_>>();
                Some(DragColumnPresentation {
                    column: local_column,
                    groups: group_rects,
                    panels: panel_rects,
                    tab_strips: strip_rects,
                    tab_headers: header_rects,
                    tab_overflows: overflow_rects,
                    presentations,
                    group_ids,
                    mode: column_model.mode,
                    rail_items: controller.layout().column_display_order(column).unwrap_or_default(),
                    splitters,
                    vertical_overflow,
                    metrics: controller.metrics(),
                })
            })
        {
            let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
            root = root.child(DockCompositionView::render_column(
                snapshot.column,
                &snapshot.groups,
                &snapshot.panels,
                &snapshot.tab_strips,
                &snapshot.tab_headers,
                &snapshot.tab_overflows,
                &snapshot.presentations,
                &snapshot.group_ids,
                snapshot.mode,
                &snapshot.rail_items,
                None,
                None,
                &[],
                snapshot.metrics,
                None,
                toggle_menu,
                true,
                &snapshot.splitters,
                &snapshot.vertical_overflow,
                None,
                self.controller.clone(),
                self.theme,
                None,
                true,
                None,
                None,
            ));
            root = root.child(DockAreaView::render_header(
                format!("drag-host-column-header-{:?}", snapshot.column.column),
                snapshot.column.rect,
                &[snapshot.column],
                None,
                snapshot.metrics,
                self.theme,
            ));
        } else if let Some(group) = self.group
            && let Some(snapshot) = self.controller.update(cx, |controller, _| {
                let (source_host, source_window_id, panel) = {
                    let session = controller.drag().host_session()?;
                    if !matches!(
                        session.payload(),
                        DragPayload::Group { group: active } if active == group
                    ) {
                        return None;
                    }
                    let panel = session.transferred_panel()?;
                    if !session.panel_view_source_detached(panel) {
                        return None;
                    }
                    (session.source_host(), session.source_window_id()?, panel)
                };
                let rects = match source_host {
                    DragSourceHost::Main => controller.geometry().ok()?.clone(),
                    DragSourceHost::Floating { group: floating } => {
                        controller.floating_geometry(source_window_id, floating).ok()?.clone()
                    }
                };
                let group_rect = rects
                    .groups
                    .iter()
                    .find(|rect| rect.group == group)
                    .copied()?;
                let strip = rects
                    .tab_strips
                    .iter()
                    .find(|strip| strip.group == group)
                    .copied()?;
                let headers = rects
                    .tab_headers
                    .iter()
                    .filter(|header| header.group == group)
                    .copied()
                    .collect::<Vec<_>>();
                let overflow = rects
                    .tab_overflows
                    .iter()
                    .find(|overflow| overflow.group == group)
                    .cloned();
                let ids = HashSet::from([group]);
                let presentations = collect_group_presentations(
                    controller.layout(),
                    controller.registry(),
                    &ids,
                    Some(&rects),
                );
                let presentation = presentations.get(&group)?.clone();
                let panel_view = controller.panel_view_for_drag(panel);
                let panel_rect = rects
                    .panels
                    .iter()
                    .find(|rect| rect.panel == panel)
                    .copied();
                let panel = panel_rect.zip(panel_view);
                let column = controller.layout().nodes().find_map(|(id, node)| {
                    matches!(
                        node,
                        Node::Column(value) if value.groups.contains(&group)
                    )
                    .then_some(id)
                });
                Some(DragGroupPresentation {
                    group: group_rect,
                    strip,
                    headers,
                    overflow,
                    presentation,
                    column,
                    panel,
                    metrics: controller.metrics(),
                })
            })
        {
            let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
            root = root.child(DockGroupView::render_core(
                snapshot.column,
                snapshot.group,
                snapshot.strip,
                &snapshot.headers,
                snapshot.overflow,
                &snapshot.presentation,
                snapshot.group.rect.origin,
                snapshot.metrics,
                0.0,
                self.controller.clone(),
                self.theme,
                toggle_menu,
            ));
            if let Some(panel) = snapshot.panel {
                root = root.child(PanelHost::render(
                    vec![(panel.0, Some(panel.1))],
                    snapshot.group.rect.origin,
                    self.theme,
                ));
            }
        } else if let Some(panel) = self.panel
            && let Some(view) = self.controller.update(cx, |controller, _| {
                controller.panel_view_for_drag(panel)
            })
        {
            root = root.child(div().size_full().overflow_hidden().child(view));
        }

        root
    }
}

fn floating_group_for_panel(
    layout: &dock_core::Layout,
    panel: PanelId,
) -> Option<FloatingGroupId> {
    fn contains_panel(layout: &dock_core::Layout, node: NodeId, panel: PanelId) -> bool {
        match layout.node(node) {
            Some(Node::Split(split)) => split
                .children
                .iter()
                .any(|child| contains_panel(layout, *child, panel)),
            Some(Node::Column(column)) => column
                .groups
                .iter()
                .any(|group| contains_panel(layout, *group, panel)),
            Some(Node::Tabs(tabs)) => tabs
                .children
                .iter()
                .any(|child| contains_panel(layout, *child, panel)),
            Some(Node::Panel(value)) => value.panel_id == panel,
            None => false,
        }
    }

    layout.floating_groups().find_map(|(group_id, group)| {
        let area = layout.dock_area(group.area)?;
        contains_panel(layout, area.root, panel).then_some(group_id)
    })
}

fn valid_scale(scale: f32) -> Option<f32> {
    (scale.is_finite() && scale > 0.0).then_some(scale)
}
