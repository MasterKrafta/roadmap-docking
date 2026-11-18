//! Runtime owner of temporary native windows used to present dragged dock content.
//!
//! A DragHost is deliberately independent from Core Layout and the persistent
//! FloatingWindowManager. It hosts the existing source presentation and routes
//! drop feedback to the persistent Main/Floating window under the cursor.

use std::{collections::{HashMap, HashSet}, time::Duration};

use gpui::{
    AppContext, Bounds, Context, Entity, Window, WindowBounds, WindowHandle, WindowId, WindowKind,
    WindowOptions, point, px, size,
};
use platform::{NativeError, NativeKey, NativeWindowId, NativeWindowOps, SystemNativeWindowOps};

use dock_core::{
    ColumnRect, CommandValue, DockSide, DragDropAction, DragHandle, DragPayload, DragState,
    DropTarget, FloatingGroupId, Node, NodeId, PanelId, Point, Rect, Size,
};

use crate::{
    components::{
        floating::{
            floating_group_and_column_for_group, floating_group_column_for_panel, FloatingPartHost,
        },
    },
    floating_manager::{floating_window_chain_direction, DockingWindowAtPoint},
    geometry::DockGeometry,
    window_chain::WindowGridDirection,
    DockController, DockTheme, DockWindowRole, DragHostId, DragSourceHost, FloatingWindowManager,
};


#[path = "drag_host_manager/active_drag_host.rs"]
mod active_drag_host;
use active_drag_host::ActiveDragHost;

#[path = "drag_host_manager/drag_host_handoff.rs"]
mod drag_host_handoff;
use drag_host_handoff::DragHostHandoff;

#[path = "drag_host_manager/drag_host_error.rs"]
mod drag_host_error;
pub use drag_host_error::DragHostError;

#[path = "drag_host_manager/drag_host_surface.rs"]
mod drag_host_surface;
pub(crate) use drag_host_surface::DragHostSurface;

#[path = "drag_host_manager/drag_group_presentation.rs"]
mod drag_group_presentation;
use drag_group_presentation::DragGroupPresentation;

#[path = "drag_host_manager/drag_dock_area_presentation.rs"]
mod drag_dock_area_presentation;
use drag_dock_area_presentation::DragDockAreaPresentation;


fn source_layout_rects(
    controller: &mut DockController,
    source_host: DragSourceHost,
    source_window_id: WindowId,
) -> Option<dock_core::LayoutRects> {
    match source_host {
        DragSourceHost::Main => Some(controller.geometry().ok()?.clone()),
        DragSourceHost::Floating { group }
        | DragSourceHost::FloatingPart { group, .. } => {
            Some(controller.floating_geometry(source_window_id, group).ok()?.clone())
        }
    }
}

fn group_bounds_to_floating_composition_bounds(group_bounds: Rect, header_height: f32) -> Rect {
    Rect::new(
        Point::new(
            group_bounds.origin.x,
            group_bounds.origin.y - header_height,
        ),
        Size::new(
            group_bounds.size.width,
            group_bounds.size.height + header_height,
        ),
    )
}

fn dock_area_drag_uses_free_area(target_window: Option<DockingWindowAtPoint>) -> bool {
    matches!(
        target_window,
        None | Some(DockingWindowAtPoint::ForeignWindow)
    )
}

fn floating_window_chain_target(
    source_host: DragSourceHost,
    handle: Option<DragHandle>,
    target: Option<DropTarget>,
    target_role: Option<DockWindowRole>,
) -> Option<(FloatingGroupId, WindowGridDirection)> {
    if !matches!(
        handle,
        Some(DragHandle::PanelTab)
            | Some(DragHandle::GroupGrip)
            | Some(DragHandle::DockAreaHeader)
    ) {
        return None;
    }

    let Some(target) = target else {
        return None;
    };
    let group = match target_role {
        Some(DockWindowRole::Floating { group })
        | Some(DockWindowRole::FloatingPart { group, .. }) => group,
        _ => return None,
    };
    let source_group = match source_host {
        DragSourceHost::Main => None,
        DragSourceHost::Floating { group } | DragSourceHost::FloatingPart { group, .. } => {
            Some(group)
        }
    };
    if source_group == Some(group) {
        // Sibling Header/Group HWNDs in one Core Floating composition are not
        // separate WindowGrid targets. Their edge drops stay in Core's resolver.
        return None;
    }
    // A content drop on any side of another Floating window creates a separate
    // FloatingGroup and joins its native HWND to WindowGrid. Plain Top/Bottom
    // drops in Main still use the regular Core Group split path.
    floating_window_chain_direction(target).map(|direction| (group, direction))
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
            false,
            false,
            false,
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
            false,
            false,
            false,
            cx,
        )
    }

    /// Opens DragHost for a PanelTab tear-off that can be promoted to Floating
    /// after release without replacing its native window.
    pub fn open_panel_tab_at_cursor(
        &mut self,
        panel: PanelId,
        grab_offset: Point,
        logical_size: Size,
        promote_on_float: bool,
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
            promote_on_float,
            true,
            true,
            cx,
        )
    }

    /// Opens DragHost with every cached PanelView belonging to the source Group.
    ///
    /// The active tab is first so the temporary transport window can present the
    /// Group's visible content while its permanent Group host is prepared.
    pub fn open_group_at_cursor(
        &mut self,
        group: NodeId,
        panels: Vec<PanelId>,
        grab_offset: Point,
        logical_size: Size,
        promote_on_float: bool,
        cx: &mut Context<Self>,
    ) -> Result<DragHostId, DragHostError> {
        let can_transfer = self.controller.update(cx, |controller, _| {
            controller.can_begin_group_view_transfer(group, &panels)
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
            Some(group),
            None,
            promote_on_float,
            true,
            false,
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
            Some(side),
            true,
            false,
            false,
            cx,
        )
    }

    fn open_at_cursor_with_content(
        &mut self,
        grab_offset: Point,
        logical_size: Size,
        panels: Vec<PanelId>,
        group: Option<NodeId>,
        dock_area: Option<DockSide>,
        promote_on_float: bool,
        remove_window_frame: bool,
        render_detached_tab: bool,
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
        let defer_layout_events_until_parts_ready =
            dock_area.is_some() || promote_on_float;
        let surface_panel = panels.first().copied();
        let window = cx
            .open_window(options, move |_, app| {
                app.new(|_| DragHostSurface {
                    theme,
                    controller,
                    controller_observed: false,
                    panel: surface_panel,
                    group,
                    dock_area,
                    render_detached_tab,
                    drag_content_size: logical_size,
                    handoff_placeholder: false,
                    promoted_part: None,
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
        let frame_state = if remove_window_frame {
            match self.native_ops.remove_window_frame(native_window) {
                Ok(state) => {
                    if let Err(error) = window.update(cx, |_, window, app| {
                        window.bounds_changed(app);
                    }) {
                        self.discard_unregistered_window(&window, native_window, cx);
                        return Err(DragHostError::WindowUpdate(error.to_string()));
                    }
                    Some(state)
                }
                Err(NativeError::Unsupported) => None,
                Err(error) => {
                    self.discard_unregistered_window(&window, native_window, cx);
                    return Err(DragHostError::Native(error));
                }
            }
        } else {
            None
        };
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
            let transferred = match (group, dock_area, panels.as_slice()) {
                (Some(group), None, panels) => {
                    controller.begin_group_view_transfer(group, panels)
                }
                (None, Some(side), panels) => {
                    controller.begin_dock_area_view_transfer(side, panels)
                }
                (None, None, [panel]) => controller.begin_panel_view_transfer(*panel),
                (None, None, []) => true,
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
        let frame_was_removed = frame_state.is_some();
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
            frame_state,
            frame_was_removed,
            visible,
            drop_target: None,
            target_role: None,
            free_area_bounds: None,
            defer_layout_events_until_parts_ready,
            pending_release_cursor: None,
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
        self.native_ops
            .move_window_to_top(active.native_window, position)?;
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

        // DragHost tracks the global pointer independently from GPUI focus.
        // Foreground activation is not a reliable drag boundary: moving over a
        // different DockHost can change the active HWND while the left button
        // is still held. Keep the composition moving until mouse-up or Escape.

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
        let target_window = floating_manager.update(cx, |manager, manager_cx| {
            manager.docking_window_at_point(
                cursor,
                native_drag_host,
                excluded_source_window,
                manager_cx,
            )
        });
        let source_point = floating_manager.update(cx, |manager, manager_cx| {
            manager.core_point_for_drag_source(
                source_host,
                source_window_id,
                cursor,
                manager_cx,
            )
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
        // Main DockArea drags always use the complete composition root as source.
        let source = match payload {
            DragPayload::DockArea { side } => self.controller.update(cx, |controller, _| {
                let root = controller.layout().main_area(side)?.root;
                Some(dock_core::DropSource::Composition { root })
            }),
            _ => crate::input::drag::drop_source(payload),
        };

        let update: Result<(Option<DropTarget>, Option<Rect>), String> =
            self.controller.update(cx, |controller, controller_cx| {
            let target_result: Result<Option<DropTarget>, dock_core::ComputeRectsError> =
                if matches!(payload, DragPayload::DockArea { .. })
                    && (source.is_none() || dock_area_drag_uses_free_area(target_window))
                {
                    // A complete Main DockArea has no docking target outside this
                    // application's Main/Floating hosts. Releasing over another
                    // application's window is therefore a FreeArea tear-off too.
                    Ok(Some(DropTarget::FreeArea))
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
                            }) | Some(DockingWindowAtPoint::FloatingPart {
                                group,
                                window_id,
                                point,
                                ..
                            }),
                        ) => controller
                            .prepare_floating_window_drop_resolver(window_id, group, source)
                            .and_then(|resolver| resolver.resolve(point)),
                        (Some(_), Some(DockingWindowAtPoint::ForeignWindow)) => Ok(None),
                        // No managed Dock window is under the cursor. This is the
                        // desktop/free-space destination for both Main and Floating
                        // sources. A Floating snapshot has no Canvas/FreeArea zone,
                        // so asking its resolver here would silently cancel the tear-off.
                        (Some(_), None) => Ok(Some(DropTarget::FreeArea)),
                        (None, _) => Ok(None),
                    }
                };
            let target = target_result.map_err(|error| error.to_string())?;

            // A resolved dock target supplies coordinates in that target Host.
            // FreeArea has no target Host, so keep the pointer in the original
            // source Host's local coordinate system instead of mixing Main/Floating
            // coordinate spaces in the DragDropRequest.
            let pointer_point = if target == Some(DropTarget::FreeArea) {
                // FreeArea bounds are calculated in the original source Host's
                // coordinate space, never in a window under the release cursor.
                source_point
            } else {
                target_window
                    .and_then(|target| match target {
                        DockingWindowAtPoint::Main { point, .. }
                        | DockingWindowAtPoint::Floating { point, .. }
                        | DockingWindowAtPoint::FloatingPart { point, .. } => Some(point),
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

            let needs_floating_bounds = target == Some(DropTarget::FreeArea)
                || floating_window_chain_target(
                    source_host,
                    Some(handle),
                    target,
                    target_window.and_then(DockingWindowAtPoint::role),
                )
                .is_some();
            let local_free_area_bounds = if needs_floating_bounds {
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
            floating_manager.update(cx, |manager, manager_cx| {
                manager.screen_bounds_for_drag_source(source_host, bounds, manager_cx)
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
        // Remember the foreground window at the exact release point. The OS may
        // restore it (or Main) after DragHost closes; a different window appearing
        // afterward means the user deliberately changed focus during handoff.
        let foreground_at_release = self.native_ops.foreground_window().ok().flatten();
        let target = active.drop_target;
        let free_area_bounds = active.free_area_bounds;
        let panel = active.panels.first().copied();
        let defer_layout_events_until_parts_ready =
            active.defer_layout_events_until_parts_ready;
        let (drag_handle, source_host) = self.controller.update(cx, |controller, _| {
            (
                controller.drag().session().handle(),
                controller.drag().host_session().map(|session| session.source_host()),
            )
        });
        let chain_join = source_host.and_then(|source_host| {
            floating_window_chain_target(
                source_host,
                drag_handle,
                target,
                active.target_role,
            )
        });
        let restored_free_area_bounds = if defer_layout_events_until_parts_ready
            && (target == Some(DropTarget::FreeArea) || chain_join.is_some())
        {
            match self.restore_active_window_frame(id, cx) {
                Ok(bounds) => Some(bounds),
                Err(error) => {
                    tracing::warn!(?id, %error, "could not restore DragHost frame before Floating promotion");
                    None
                }
            }
        } else {
            None
        };
        let free_area_bounds = restored_free_area_bounds.or(free_area_bounds);
        let needs_column_header = (target == Some(DropTarget::FreeArea) || chain_join.is_some())
            && matches!(
                drag_handle,
                Some(DragHandle::PanelTab | DragHandle::GroupGrip)
            );
        let free_area_bounds = if needs_column_header {
            let header_height = self.controller.update(cx, |controller, _| {
                controller.metrics().column_header_height
            });
            free_area_bounds.map(|bounds| {
                group_bounds_to_floating_composition_bounds(bounds, header_height)
            })
        } else {
            free_area_bounds
        };
        let (finished, created_groups) = self.controller.update(cx, |controller, controller_cx| {
            let Some(preview) = controller.drag().session().preview() else {
                let _ = controller.cancel_pointer_interaction();
                let _ = controller.drag_mut().session_mut().reset();
                controller.drag_mut().clear_resolver();
                controller.drag_mut().clear_layout_version();
                controller_cx.notify();
                return (false, Vec::new());
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

            let mut created_groups = Vec::new();
            if let Some(mut request) = request {
                if let Some((anchor, direction)) = chain_join {
                    // Content side drops create an independent Core FloatingGroup.
                    // The native HWND is linked to the target only after that group
                    // exists; the DragHost bounds are the requested floating bounds.
                    request.target = Some(DropTarget::FreeArea);
                    request.floating_bounds = free_area_bounds;
                    if request.floating_bounds.is_none() {
                        tracing::warn!(
                            ?id,
                            ?anchor,
                            ?direction,
                            "WindowGrid side drop has no valid DragHost bounds"
                        );
                    }
                } else if request.target == Some(DropTarget::FreeArea) {
                    request.floating_bounds = free_area_bounds;
                    if request.floating_bounds.is_none() {
                        tracing::debug!(?id, "DragHost FreeArea drop has no valid screen bounds");
                    }
                }

                let can_resolve = request.target != Some(DropTarget::FreeArea)
                    || request.floating_bounds.is_some();
                if can_resolve {
                    let metrics = controller.metrics();
                    let action = controller.layout().resolve_drag_drop(&request, &metrics);
                    match action {
                        Ok(DragDropAction::NoOp) => {}
                        Ok(DragDropAction::Command(command)) => {
                            let result = if defer_layout_events_until_parts_ready {
                                // Delay LayoutChanged publication until permanent
                                // Header/Group hosts can reconcile the new Core layout.
                                controller.dispatch(command)
                            } else {
                                controller.dispatch_in(command, controller_cx)
                            };
                            match result {
                                Ok(outcome) => {
                                    match outcome.actual_value {
                                        Some(CommandValue::FloatingGroups(groups)) => {
                                            created_groups = groups;
                                        }
                                        Some(CommandValue::FloatingGroup(group)) => {
                                            created_groups = vec![group];
                                        }
                                        _ => {}
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(?id, %error, "DragHost drop command was rejected");
                                }
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
            (true, created_groups)
        });

        // Register the complete source-ordered chain before its separate
        // Header/Group hosts are reconciled for every Core FloatingGroup.
        if finished && created_groups.len() > 1 && chain_join.is_none() {
            let registered = self.floating_manager.as_ref().is_some_and(|manager| {
                manager.update(cx, |manager, _| {
                    manager.register_window_chain(&created_groups)
                })
            });
            if !registered {
                tracing::error!(
                    ?id,
                    groups = created_groups.len(),
                    "could not register the Core-created Main DockArea Window Chain"
                );
            }
        }

        if let Some(panel) = panel
            && !finished
        {
            tracing::debug!(?id, ?panel, "DragHost release was cancelled; source view will be restored");
        }

        // FreeArea and WindowGrid side drops must create a new Core FloatingGroup.
        // Never resolve the still-source-owned Panel back to its original group if
        // the request was skipped, rejected, or produced no detached composition.
        let requires_new_floating_group =
            target == Some(DropTarget::FreeArea) || chain_join.is_some();
        if finished && requires_new_floating_group && created_groups.is_empty() {
            tracing::warn!(
                ?id,
                ?target,
                ?drag_handle,
                has_free_area_bounds = free_area_bounds.is_some(),
                "Floating tear-off did not create a new FloatingGroup; retaining the source layout"
            );
            self.flush_controller_events(cx);
            if let Err(error) = self.close(id, cx) {
                tracing::warn!(?id, %error, "could not close DragHost after rejected Floating tear-off");
            }
            return;
        }

        let floating_target = if finished {
            panel.and_then(|panel| {
                self.controller.update(cx, |controller, _| {
                    let group = floating_group_for_panel(controller.layout(), panel)?;
                    (!requires_new_floating_group || created_groups.contains(&group))
                        .then_some(group)
                })
            })
        } else {
            None
        };

        if let Some(target_group) = floating_target {
            // Main-origin GroupGrip and detached PanelTab gestures can reuse
            // their DragHost HWND as the new permanent Group host. PanelTab promotion
            // works from Main and Floating; GroupGrip still uses the direct native HWND
            // path when its source is Floating. Resolve the resulting Group by its
            // preserved NodeId (GroupGrip) or transferred PanelId (PanelTab).
            // A PanelTab merged into an existing Group is rejected by adoption because
            // that Group already has its own HWND; the safe handoff below serves it.
            if defer_layout_events_until_parts_ready
                && (
                    drag_handle == Some(DragHandle::PanelTab)
                    || (source_host == Some(DragSourceHost::Main)
                        && drag_handle == Some(DragHandle::GroupGrip))
                )
            {
                let destination = self.controller.update(cx, |controller, _| {
                    let session = controller.drag().host_session()?;
                    match session.payload() {
                        DragPayload::Group { group } => {
                            let (owner, column) = floating_group_and_column_for_group(
                                controller.layout(),
                                group,
                            )?;
                            Some((owner, column, group))
                        }
                        DragPayload::Panel { panel } => {
                            floating_group_column_for_panel(controller.layout(), panel)
                        }
                        _ => None,
                    }
                });
                if let Some((owner, column, group)) = destination
                    && owner == target_group
                    && self.promote_drag_host_content_to_floating(
                        id,
                        target_group,
                        column,
                        group,
                        cx,
                    )
                {
                    // Registration happens before publication, so reconciliation sees
                    // the adopted HWND and creates only the missing Header windows.
                    self.flush_controller_events(cx);
                    if let Some((anchor, direction)) = chain_join {
                        self.join_created_groups_to_chain(
                            id,
                            anchor,
                            &created_groups,
                            direction,
                            cx,
                        );
                    }
                    return;
                }
            }

            // Tab-strip merges and any unsupported promotion retain the existing
            // safe two-phase PanelView handoff to the surviving destination Group host.
            if let Some((anchor, direction)) = chain_join {
                // Resolve the new Core FloatingGroup before linking its WindowGrid
                // members, then hand the transported PanelViews to their Group hosts.
                self.flush_controller_events(cx);
                self.join_created_groups_to_chain(
                    id,
                    anchor,
                    &created_groups,
                    direction,
                    cx,
                );

                if !self.begin_floating_handoff(id, target_group, foreground_at_release, cx) {
                    tracing::warn!(
                        ?id,
                        ?target_group,
                        "could not prepare the DragHost-to-Group-host handoff after WindowGrid insertion"
                    );
                    if let Err(error) = self.close(id, cx) {
                        tracing::warn!(?id, %error, "could not close DragHost after handoff setup failed");
                    }
                }
                return;
            }

            if defer_layout_events_until_parts_ready {
                self.flush_controller_events(cx);
            }

            if !self.begin_floating_handoff(id, target_group, foreground_at_release, cx) {
                tracing::warn!(
                    ?id,
                    ?target_group,
                    "could not prepare the DragHost-to-Group-host handoff; closing DragHost"
                );
                if let Err(error) = self.close(id, cx) {
                    tracing::warn!(?id, %error, "could not close DragHost after handoff setup failed");
                }
            }
            return;
        }

        if defer_layout_events_until_parts_ready {
            self.flush_controller_events(cx);
        }
        if let Err(error) = self.close(id, cx) {
            tracing::warn!(?id, %error, "could not close DragHost after release");
        }
    }

    fn promote_drag_host_content_to_floating(
        &mut self,
        id: DragHostId,
        floating_group: FloatingGroupId,
        column: NodeId,
        group: NodeId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((window, window_id, native_window, panels, frame_was_removed, frame_state)) =
            self.active
                .as_ref()
                .filter(|active| active.id == id)
                .map(|active| (
                    active.window,
                    active.window_id,
                    active.native_window,
                    active.panels.clone(),
                    active.frame_was_removed,
                    active.frame_state,
                ))
        else {
            return false;
        };
        let Some(manager) = self.floating_manager.clone() else {
            return false;
        };

        // FreeArea / WindowGrid releases measure the normal window frame before
        // building Core bounds. Restore the DragHost's original borderless style
        // before reusing it as a permanent Group part.
        if frame_was_removed && frame_state.is_none() {
            let restored_frame = match self.native_ops.remove_window_frame(native_window) {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(
                        ?id,
                        %error,
                        "could not restore the borderless style before promoting DragHost"
                    );
                    return false;
                }
            };
            if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
                active.frame_state = Some(restored_frame);
            }
            if let Err(error) = window.update(cx, |_, window, app| window.bounds_changed(app)) {
                tracing::warn!(
                    ?id,
                    %error,
                    "could not refresh the promoted DragHost window bounds"
                );
                return false;
            }
        }

        let controller = self.controller.clone();
        let theme = self.theme;
        let manager_weak = manager.downgrade();
        let promoted = window.update(cx, |surface, _, host_cx| {
            if surface.promoted_part.is_some() {
                return false;
            }
            surface.promoted_part = Some(host_cx.new(|_| {
                FloatingPartHost::new_group(
                    controller,
                    floating_group,
                    column,
                    group,
                    theme,
                    manager_weak,
                )
            }));
            host_cx.notify();
            true
        });
        if !matches!(promoted, Ok(true)) {
            return false;
        }

        let adopted = manager.update(cx, |manager, manager_cx| {
            manager.adopt_drag_host_group_window(
                group,
                floating_group,
                window,
                window_id,
                native_window,
                manager_cx,
            )
        });
        if !adopted {
            let _ = window.update(cx, |surface, _, host_cx| {
                surface.promoted_part = None;
                host_cx.notify();
            });
            return false;
        }

        // The root now renders the same Group in the same HWND. Release the
        // transport claim only after switching the surface, then let Main render
        // the committed Core layout without its old Group placeholder.
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_transport_host(id);
            for panel in &panels {
                controller.finish_panel_view_transfer(*panel);
            }
            controller.drag_mut().finish_host_session();
            controller_cx.notify();
        });

        if let Some(mut active) = self.active.take() {
            if active.id == id {
                drop(active.source_detach_subscription.take());
            } else {
                self.active = Some(active);
                return false;
            }
        }
        cx.notify();
        true
    }

    fn join_created_groups_to_chain(
        &self,
        id: DragHostId,
        anchor: FloatingGroupId,
        groups: &[FloatingGroupId],
        direction: WindowGridDirection,
        cx: &mut Context<Self>,
    ) {
        if groups.is_empty() {
            tracing::warn!(
                ?id,
                ?anchor,
                "Core did not return FloatingGroups for a WindowGrid side drop"
            );
            return;
        }

        let joined = self.floating_manager.as_ref().map(|manager| {
            manager.update(cx, |manager, manager_cx| {
                manager.join_window_chain_at(anchor, groups, direction, manager_cx)
            })
        });

        match joined {
            Some(Ok(())) => {}
            Some(Err(error)) => tracing::warn!(
                ?id,
                ?anchor,
                %error,
                "could not join the newly floated content windows to the target WindowGrid"
            ),
            None => tracing::warn!(
                ?id,
                ?anchor,
                "FloatingWindowManager was unavailable for the WindowGrid side drop"
            ),
        }
    }

    /// Restores the native frame before committing a content tear-off as a Floating composition.
    /// The same client rectangle stays on screen; only the outer frame changes.
    fn restore_active_window_frame(
        &mut self,
        id: DragHostId,
        cx: &mut Context<Self>,
    ) -> Result<Rect, String> {
        let Some(active) = self.active.as_ref().filter(|active| active.id == id) else {
            return Err("DragHost is no longer active".into());
        };
        let native_window = active.native_window;
        let window = active.window;
        let frame_state = active.frame_state;

        if let Some(frame_state) = frame_state {
            self.native_ops
                .restore_window_frame(native_window, frame_state)
                .map_err(|error| error.to_string())?;
            if let Some(active) = self.active.as_mut().filter(|active| active.id == id) {
                active.frame_state = None;
            }
            window
                .update(cx, |_, window, app| window.bounds_changed(app))
                .map_err(|error| error.to_string())?;
        }

        let window_rect = self.native_ops.window_rect(native_window)
            .map_err(|error| error.to_string())?;
        let client_size = self.native_ops.client_size(native_window)
            .map_err(|error| error.to_string())?;
        let scale = self.native_ops.scale_factor(native_window)
            .map_err(|error| error.to_string())?;
        DockGeometry::core_bounds_from_native(window_rect, client_size, scale)
            .ok_or_else(|| "could not convert restored DragHost bounds to DockCore coordinates".into())
    }

    fn begin_floating_handoff(
        &mut self,
        id: DragHostId,
        target_group: FloatingGroupId,
        foreground_at_release: Option<NativeWindowId>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(active) = self.active.as_ref().filter(|active| active.id == id) else {
            return false;
        };
        let panels = active.panels.clone();
        let window = active.window.clone();

        let required_panels_by_group = self.controller.update(cx, |controller, _| {
            let layout = controller.layout();
            let transferred = panels.iter().copied().collect::<HashSet<_>>();
            let affected_groups = layout
                .nodes()
                .filter_map(|(group, node)| {
                    let dock_core::Node::Tabs(tabs) = node else {
                        return None;
                    };
                    tabs.children
                        .iter()
                        .any(|child| {
                            matches!(
                                layout.node(*child),
                                Some(dock_core::Node::Panel(panel))
                                    if transferred.contains(&panel.panel_id)
                            )
                        })
                        .then_some(group)
                })
                .collect::<HashSet<_>>();
            let metrics = controller.metrics();
            let mut by_group = HashMap::<FloatingGroupId, Vec<PanelId>>::new();

            // Each destination GroupHost mounts only its current active PanelView.
            // Require those actual views, not inactive tabs that stay in the shared cache.
            for group_node in affected_groups {
                let active_panel = match layout.node(group_node) {
                    Some(dock_core::Node::Tabs(tabs)) => tabs
                        .active_child()
                        .and_then(|child| match layout.node(child) {
                            Some(dock_core::Node::Panel(panel)) => Some(panel.panel_id),
                            _ => None,
                        }),
                    _ => None,
                };
                let Some(active_panel) = active_panel else {
                    continue;
                };
                let Some(floating_group) = floating_group_for_panel(layout, active_panel) else {
                    continue;
                };
                let visible = layout
                    .compute_floating_rects(floating_group, &metrics)
                    .ok()
                    .is_some_and(|rects| {
                        rects.panels.iter().any(|rect| rect.panel == active_panel)
                    });
                if visible {
                    by_group
                        .entry(floating_group)
                        .or_default()
                        .push(active_panel);
                }
            }
            if by_group.is_empty() {
                // A collapsed Group may have no active PanelRect to mount, but its
                // Group host still needs to render once before DragHost is closed.
                by_group.insert(target_group, Vec::new());
            }
            by_group
        });

        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return false;
        };
        active.handoff = Some(DragHostHandoff {
            primary_target_group: target_group,
            required_panels_by_group,
            rendered_panels: HashSet::new(),
            foreground_at_release,
            source_placeholder_rendered: false,
            source_released: false,
        });

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

        // The next-frame callback already confirms that the empty source
        // placeholder was rendered. Try the handoff immediately instead of
        // always waiting for the first 8 ms polling tick.
        if !self.try_release_handoff_source(id, cx) {
            return;
        }

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
    /// The temporary DragHost releases its PanelViews only after rendering an empty
    /// placeholder; permanent Group hosts are then shown before their next frame.
    fn try_release_handoff_source(
        &mut self,
        id: DragHostId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((primary_target_group, panels, placeholder_rendered, source_released)) = self
            .active
            .as_ref()
            .filter(|active| active.id == id)
            .and_then(|active| {
                let handoff = active.handoff.as_ref()?;
                Some((
                    handoff.primary_target_group,
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
            self.controller.update(cx, |controller, controller_cx| {
                for panel in &panels {
                    controller.finish_panel_view_transfer(*panel);
                }
                controller.drag_mut().finish_host_session();
                controller_cx.notify();
            });
            if let Some(active) = self.active.as_mut().filter(|active| active.id == id)
                && let Some(handoff) = active.handoff.as_mut()
            {
                handoff.source_released = true;
            }
        }

        let Some(expected_groups) = self.pending_handoff_groups(primary_target_group) else {
            return true;
        };
        let target_shown = self
            .floating_manager
            .as_ref()
            .is_some_and(|manager| {
                manager.update(cx, |manager, manager_cx| {
                    manager.show_handoff_target(
                        primary_target_group,
                        &expected_groups,
                        manager_cx,
                    )
                })
            });
        !target_shown
    }

    pub(crate) fn pending_handoff_panels(
        &self,
        target_group: FloatingGroupId,
    ) -> Option<Vec<PanelId>> {
        let active = self.active.as_ref()?;
        let handoff = active.handoff.as_ref()?;
        if !handoff.source_released {
            return None;
        }
        handoff.required_panels_by_group.get(&target_group).cloned()
    }

    /// Returns every FloatingGroup whose permanent Group hosts belong to this
    /// released transfer. A DockArea tear-off may need several destinations.
    pub(crate) fn pending_handoff_groups(
        &self,
        primary_target_group: FloatingGroupId,
    ) -> Option<Vec<FloatingGroupId>> {
        let active = self.active.as_ref()?;
        let handoff = active.handoff.as_ref()?;
        if !handoff.source_released
            || handoff.primary_target_group != primary_target_group
        {
            return None;
        }

        let mut groups = handoff
            .required_panels_by_group
            .keys()
            .copied()
            .collect::<Vec<_>>();
        if !groups.contains(&primary_target_group) {
            groups.push(primary_target_group);
        }
        Some(groups)
    }

    pub(crate) fn complete_floating_handoff(
        &mut self,
        target_group: FloatingGroupId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
        let completion = {
            let Some(active) = self.active.as_mut() else {
                return;
            };
            let Some(handoff) = active.handoff.as_mut() else {
                return;
            };
            if !handoff.source_released {
                return;
            }
            let required_for_group = handoff
                .required_panels_by_group
                .get(&target_group)
                .cloned();
            if let Some(required_for_group) = required_for_group {
                for panel in rendered_panels {
                    if required_for_group.contains(panel) {
                        handoff.rendered_panels.insert(*panel);
                    }
                }
            } else if !handoff.required_panels_by_group.is_empty()
                || handoff.primary_target_group != target_group
            {
                return;
            }

            let complete = handoff
                .required_panels_by_group
                .values()
                .flatten()
                .all(|panel| handoff.rendered_panels.contains(panel));
            complete.then_some((
                active.id,
                active.native_window,
                handoff.primary_target_group,
                handoff.foreground_at_release,
            ))
        };
        let Some((id, closing_drag_host, primary_target_group, foreground_at_release)) = completion
        else {
            return;
        };

        if let Err(error) = self.close(id, cx) {
            tracing::warn!(
                ?id,
                ?primary_target_group,
                %error,
                "could not close DragHost after Floating Group hosts rendered transferred content"
            );
        }

        // DragHost is owned by Main and hidden asynchronously. Retry briefly
        // while it remains the foreground HWND; activate the primary destination
        // only after Windows has completed the owner-focus handoff.
        if let Some(floating_manager) = self.floating_manager.clone() {
            cx.spawn(async move |_, app| {
                let mut activated = false;
                for _ in 0..8 {
                    app.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    activated = floating_manager.update(app, |manager, _| {
                        manager.activate_handoff_target(
                            primary_target_group,
                            closing_drag_host,
                            foreground_at_release,
                        )
                    });
                    if activated {
                        break;
                    }
                }
                if !activated {
                    tracing::debug!(
                        ?primary_target_group,
                        "Floating handoff completed without activating its destination"
                    );
                }
            })
            .detach();
        }
    }

    fn flush_controller_events(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, controller_cx| {
            controller.flush_events(controller_cx);
            controller_cx.notify();
        });
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

#[cfg(test)]
mod floating_window_chain_target_tests {
    use super::{dock_area_drag_uses_free_area, floating_window_chain_target, DockingWindowAtPoint};
    use crate::{DockWindowPart, DockWindowRole, DragSourceHost, window_chain::WindowGridDirection};
    use dock_core::{DragHandle, DropTarget, DropZone, FloatingGroupId, NodeId};
    use slotmap::Key;

    #[test]
    fn tab_group_and_composition_side_drops_keep_all_four_directions() {
        let anchor = FloatingGroupId::null();
        let group = NodeId::null();
        let floating_roles = [
            Some(DockWindowRole::Floating { group: anchor }),
            Some(DockWindowRole::FloatingPart {
                group: anchor,
                part: crate::DockWindowPart::Header { column: group },
            }),
            Some(DockWindowRole::FloatingPart {
                group: anchor,
                part: crate::DockWindowPart::Group { group },
            }),
        ];
        let zones = [
            (DropZone::Left, WindowGridDirection::Left),
            (DropZone::Right, WindowGridDirection::Right),
            (DropZone::Top, WindowGridDirection::Above),
            (DropZone::Bottom, WindowGridDirection::Below),
            (DropZone::FloatingTop, WindowGridDirection::Above),
            (DropZone::FloatingBottom, WindowGridDirection::Below),
        ];

        for floating in floating_roles {
            for handle in [
                DragHandle::PanelTab,
                DragHandle::GroupGrip,
                DragHandle::DockAreaHeader,
            ] {
                for (zone, direction) in zones {
                    let target = Some(DropTarget::GroupEdge { group, zone });
                    assert_eq!(
                        floating_window_chain_target(DragSourceHost::Main, Some(handle), target, floating),
                        Some((anchor, direction)),
                        "{handle:?} {zone:?} drop should preserve its WindowGrid direction",
                    );
                }
            }
        }

    }

    #[test]
    fn flyout_and_non_side_drops_keep_core_docking_semantics() {
        let anchor = FloatingGroupId::null();
        let group = NodeId::null();
        let tabs = Some(DropTarget::GroupTabs { group, index: 0 });
        let left = Some(DropTarget::GroupEdge {
            group,
            zone: DropZone::Left,
        });
        let floating = Some(DockWindowRole::Floating { group: anchor });

        assert_eq!(
            floating_window_chain_target(DragSourceHost::Main, Some(DragHandle::FlyoutPanel), left, floating),
            None,
        );
        assert_eq!(
            floating_window_chain_target(DragSourceHost::Main, Some(DragHandle::PanelTab), tabs, floating),
            None,
        );
        assert_eq!(
            floating_window_chain_target(DragSourceHost::Main, Some(DragHandle::GroupGrip), left, Some(DockWindowRole::Main)),
            None,
        );
    }

    #[test]
    fn drops_between_parts_of_one_floating_composition_use_core_docking() {
        let source_group = FloatingGroupId::null();
        let target_column = NodeId::null();
        let target_group = NodeId::null();
        let left = Some(DropTarget::GroupEdge {
            group: target_group,
            zone: DropZone::Left,
        });
        let target_roles = [
            Some(DockWindowRole::Floating { group: source_group }),
            Some(DockWindowRole::FloatingPart {
                group: source_group,
                part: DockWindowPart::Header { column: target_column },
            }),
            Some(DockWindowRole::FloatingPart {
                group: source_group,
                part: DockWindowPart::Group { group: target_group },
            }),
        ];

        for target_role in target_roles {
            assert_eq!(
                floating_window_chain_target(
                    DragSourceHost::FloatingPart {
                        group: source_group,
                        part: DockWindowPart::Group { group: target_group },
                    },
                    Some(DragHandle::GroupGrip),
                    left,
                    target_role,
                ),
                None,
            );
        }
    }

    #[test]
    fn floating_side_drop_into_main_keeps_core_docking() {
        use slotmap::SlotMap;

        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let source_group = floating_ids.insert(());
        let group = NodeId::null();
        for (zone, _) in [
            (DropZone::Left, WindowGridDirection::Left),
            (DropZone::Right, WindowGridDirection::Right),
            (DropZone::Top, WindowGridDirection::Above),
            (DropZone::Bottom, WindowGridDirection::Below),
        ] {
            let target = Some(DropTarget::GroupEdge { group, zone });
            assert_eq!(
                floating_window_chain_target(
                    DragSourceHost::FloatingPart {
                        group: source_group,
                        part: DockWindowPart::Group { group },
                    },
                    Some(DragHandle::GroupGrip),
                    target,
                    Some(DockWindowRole::Main),
                ),
                None,
                "a Floating → Main {zone:?} drop must stay on the Core route",
            );
        }
    }

    #[test]
    fn main_side_drop_into_main_never_creates_a_window_grid_join() {
        let group = NodeId::null();
        for zone in [DropZone::Left, DropZone::Right, DropZone::Top, DropZone::Bottom] {
            assert_eq!(
                floating_window_chain_target(
                    DragSourceHost::Main,
                    Some(DragHandle::DockAreaHeader),
                    Some(DropTarget::GroupEdge { group, zone }),
                    Some(DockWindowRole::Main),
                ),
                None,
                "a Main → Main {zone:?} drop must stay on the Core route",
            );
        }
    }

    #[test]
    fn drops_into_a_different_floating_composition_still_join_window_grid() {
        use slotmap::SlotMap;

        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let moving_group = floating_ids.insert(());
        let target_group = floating_ids.insert(());
        let group = NodeId::null();
        let left = Some(DropTarget::GroupEdge {
            group,
            zone: DropZone::Left,
        });

        assert_eq!(
            floating_window_chain_target(
                DragSourceHost::FloatingPart {
                    group: moving_group,
                    part: DockWindowPart::Header { column: group },
                },
                Some(DragHandle::PanelTab),
                left,
                Some(DockWindowRole::FloatingPart {
                    group: target_group,
                    part: DockWindowPart::Group { group },
                }),
            ),
            Some((target_group, WindowGridDirection::Left)),
        );
    }

    #[test]
    fn dock_area_header_treats_unmanaged_windows_as_free_area() {
        use dock_core::Point;

        assert!(dock_area_drag_uses_free_area(None));
        assert!(dock_area_drag_uses_free_area(Some(
            DockingWindowAtPoint::ForeignWindow
        )));
        assert!(!dock_area_drag_uses_free_area(Some(
            DockingWindowAtPoint::Main { point: Point::ZERO }
        )));
    }
}

#[cfg(test)]
mod floating_group_drag_bounds_tests {
    use dock_core::{Point, Rect, Size};

    use super::group_bounds_to_floating_composition_bounds;

    #[test]
    fn temporary_group_bounds_become_composition_bounds_without_moving_group_origin() {
        let temporary_group = Rect::new(
            Point::new(420.0, 280.0),
            Size::new(640.0, 480.0),
        );

        assert_eq!(
            group_bounds_to_floating_composition_bounds(temporary_group, 28.0),
            Rect::new(
                Point::new(420.0, 252.0),
                Size::new(640.0, 508.0),
            ),
        );
    }
}
