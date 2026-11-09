//! Runtime owner of temporary native windows used to present dragged dock content.
//!
//! A DragHost is deliberately independent from Core Layout and the persistent
//! FloatingWindowManager. It hosts the existing source presentation and routes
//! drop feedback to the persistent Main/Floating window under the cursor.

use std::{collections::HashSet, rc::Rc, time::Duration};

use gpui::{
    AnyView, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions, div, point, px, rgb, size,
};
use platform::{NativeError, NativeKey, NativeWindowId, NativeWindowOps, SystemNativeWindowOps};

use dock_core::{
    DockMetrics, DragDropAction, DragHandle, DragPayload, DragState, DropTarget, GroupRect, Node,
    NodeId, PanelId, PanelRect, Point, Rect, Size, TabHeaderRect, TabOverflowRect, TabStripRect,
};

use crate::{
    components::{context_menu::GroupMenuToggle, PanelHost},
    dock_components::{collect_group_presentations, DockGroupView, GroupPresentation},
    floating_manager::DockingWindowAtPoint,
    geometry::DockGeometry,
    DockController, DockTheme, DockWindowRole, DragHostId, DragSourceHost, FloatingWindowManager,
};

struct ActiveDragHost {
    id: DragHostId,
    window: WindowHandle<DragHostSurface>,
    window_id: WindowId,
    native_window: NativeWindowId,
    panel: Option<PanelId>,
    grab_offset: Point,
    visible: bool,
    drop_target: Option<DropTarget>,
    target_role: Option<DockWindowRole>,
    free_area_bounds: Option<Rect>,
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
        self.open_at_cursor_with_content(grab_offset, logical_size, None, None, cx)
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
        self.open_at_cursor_with_content(grab_offset, logical_size, Some(panel), None, cx)
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
            Some(panel),
            Some(group),
            cx,
        )
    }

    fn open_at_cursor_with_content(
        &mut self,
        grab_offset: Point,
        logical_size: Size,
        panel: Option<PanelId>,
        group: Option<NodeId>,
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
        let window = cx
            .open_window(options, move |_, app| {
                app.new(|_| DragHostSurface {
                    theme,
                    controller,
                    controller_observed: false,
                    panel,
                    group,
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
            let transferred = match (panel, group) {
                (Some(panel), Some(group)) => controller.begin_group_view_transfer(group, panel),
                (Some(panel), None) => controller.begin_panel_view_transfer(panel),
                (None, None) => true,
                (None, Some(_)) => false,
            };
            if transferred {
                controller_cx.notify();
            }
            attached && transferred
        });
        if let Some(panel) = panel
            && !host_state_ready
        {
            self.controller.update(cx, |controller, _| {
                controller.drag_mut().clear_transport_host(id);
            });
            self.discard_unregistered_window(&window, native_window, cx);
            return Err(DragHostError::PanelTransferUnavailable(panel));
        }

        let visible = panel.is_none();
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
            panel,
            grab_offset,
            visible,
            drop_target: None,
            target_role: None,
            free_area_bounds: None,
        });

        if let Some(panel) = panel {
            let controller = self.controller.clone();
            cx.observe(&controller, move |manager, controller, manager_cx| {
                let source_detached = controller.update(manager_cx, |controller, _| {
                    controller.drag().panel_view_source_detached(panel)
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
            })
            .detach();
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

        let tracking_state = self
            .controller
            .update(cx, |controller, _| {
                let Some(host_session) = controller.drag().host_session() else {
                    return None;
                };
                if host_session.transport_host() != Some(id) {
                    return None;
                }
                if host_session
                    .transferred_panel()
                    .is_some_and(|panel| !host_session.panel_view_source_detached(panel))
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
            if !self.native_ops.left_button_down().unwrap_or(true) {
                self.cancel_active_drag(id, cx);
                return false;
            }
            return true;
        };

        let Ok(cursor) = self.native_ops.cursor_position() else {
            self.cancel_active_drag(id, cx);
            return false;
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
        let source = crate::input::drag::drop_source(payload);

        let update = self.controller.update(cx, |controller, controller_cx| {
            let target_result: Result<Option<DropTarget>, dock_core::ComputeRectsError> =
                match (source, target_window) {
                    (Some(source), Some(DockingWindowAtPoint::Main { point, .. })) => controller
                        .prepare_drop_resolver(source)
                        .and_then(|resolver| resolver.resolve(point)),
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
                    (Some(source), None) => controller
                        .prepare_drop_resolver(source)
                        .map(|resolver| {
                            resolver
                                .zones()
                                .iter()
                                .any(|zone| zone.target == DropTarget::FreeArea)
                                .then_some(DropTarget::FreeArea)
                        }),
                    (None, _) => Ok(None),
                };
            let target = match target_result {
                Ok(target) => target,
                Err(error) => {
                    tracing::debug!(?id, %error, "DragHost drop-target resolution failed");
                    None
                }
            };
            // A resolved dock target supplies coordinates in that target Host.
            // FreeArea has no target Host, so keep the pointer in the original
            // source Host's local coordinate system instead of mixing Main/Floating
            // coordinate spaces in the DragDropRequest.
            let pointer_point = target_window
                .map(|target| match target {
                    DockingWindowAtPoint::Main { point, .. }
                    | DockingWindowAtPoint::Floating { point, .. } => point,
                })
                .or(source_point);
            let Some(pointer_point) = pointer_point else {
                let _ = controller.drag_mut().session_mut().set_drop_target(None);
                controller_cx.notify();
                return Some((None, None));
            };
            if let Err(error) = controller.drag_mut().session_mut().pointer_move(pointer_point) {
                tracing::debug!(?id, %error, "DragHost pointer position was rejected");
                let _ = controller.drag_mut().session_mut().set_drop_target(None);
                controller_cx.notify();
                return Some((None, None));
            }
            if let Err(error) = controller.drag_mut().session_mut().set_drop_target(target) {
                tracing::debug!(?id, %error, "DragHost target update was rejected");
                controller_cx.notify();
                return Some((None, None));
            }

            let local_free_area_bounds = if target == Some(DropTarget::FreeArea) {
                source_point.and_then(|point| {
                    crate::input::drag::floating_bounds_for_drag(
                        controller,
                        payload,
                        point,
                        source_host,
                        source_window_id,
                    )
                })
            } else {
                None
            };
            controller_cx.notify();
            Some((target, local_free_area_bounds))
        });

        let Some((target, local_free_area_bounds)) = update else {
            self.clear_drop_preview(id, cx);
            return true;
        };
        let screen_free_area_bounds = local_free_area_bounds.and_then(|bounds| {
            floating_manager.update(cx, |manager, _| {
                manager.screen_bounds_for_drag_source(source_host, bounds)
            })
        });
        if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
            active.drop_target = target;
            active.target_role = target_window.map(DockingWindowAtPoint::role);
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
        let panel = active.panel;

        let finished = self.controller.update(cx, |controller, controller_cx| {
            let Some(preview) = controller.drag().session().preview() else {
                let _ = controller.cancel_pointer_interaction();
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
                    let action = {
                        let metrics = controller.metrics();
                        controller.layout().resolve_drag_drop(&request, &metrics)
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
        if let Err(error) = self.close(id, cx) {
            tracing::warn!(?id, %error, "could not close DragHost after release");
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

    fn clear_drop_preview(&mut self, id: DragHostId, cx: &mut Context<Self>) {
        if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
            active.drop_target = None;
            active.target_role = None;
            active.free_area_bounds = None;
        }
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            controller_cx.notify();
        });
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
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        if active.id != id {
            self.active = Some(active);
            return Err(DragHostError::NotActive(id));
        }

        let still_visible = match self.native_ops.hide(active.native_window) {
            Ok(()) => false,
            Err(error) => {
                tracing::debug!(
                    ?id,
                    %error,
                    "DragHost could not be confirmed hidden during close"
                );
                true
            }
        };

        if let Err(error) = active
            .window
            .update(cx, |_, window, _| window.remove_window())
        {
            self.active = Some(ActiveDragHost {
                visible: still_visible,
                ..active
            });
            return Err(DragHostError::WindowUpdate(error.to_string()));
        }

        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_transport_host(id);
            if let Some(panel) = active.panel {
                controller.finish_panel_view_transfer(panel);
            }
            // The DragHost window is already removed. Only now may the cached
            // PanelView become mountable in its final persistent Host.
            controller.drag_mut().finish_host_session();
            controller_cx.notify();
        });
        Ok(())
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

        if let Some(group) = self.group
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

fn valid_scale(scale: f32) -> Option<f32> {
    (scale.is_finite() && scale > 0.0).then_some(scale)
}
