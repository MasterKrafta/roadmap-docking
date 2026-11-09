//! Runtime owner of temporary native windows used to present dragged dock content.
//!
//! A DragHost is deliberately independent from Core Layout and the persistent
//! FloatingWindowManager. It can host one cached PanelView during the isolated
//! transfer prototype; pointer-pipeline integration is introduced later.

use gpui::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions, div, point, px, rgb, size,
};
use platform::{NativeError, NativeWindowId, NativeWindowOps, SystemNativeWindowOps};

use dock_core::{PanelId, Point, Size};

use crate::{
    DockController, DockTheme, DockWindowRole, DragHostId,
    geometry::DockGeometry,
};

struct ActiveDragHost {
    id: DragHostId,
    window: WindowHandle<DragHostSurface>,
    window_id: WindowId,
    native_window: NativeWindowId,
    panel: Option<PanelId>,
    visible: bool,
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
            active: None,
            next_id: 1,
            native_ops: SystemNativeWindowOps,
        }
    }

    /// Returns the shared controller entity. The manager does not own its Layout.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
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
        self.open_at_cursor_with_panel(grab_offset, logical_size, None, cx)
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
        self.open_at_cursor_with_panel(grab_offset, logical_size, Some(panel), cx)
    }

    fn open_at_cursor_with_panel(
        &mut self,
        grab_offset: Point,
        logical_size: Size,
        panel: Option<PanelId>,
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
            let transferred =
                panel.is_none_or(|panel| controller.begin_panel_view_transfer(panel));
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
            visible,
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
                }
            })
            .detach();
        }

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
        let scale = valid_scale(self.native_ops.scale_factor(active.native_window)?)
            .ok_or(DragHostError::InvalidScaleFactor)?;
        let physical_grab_offset =
            DockGeometry::physical_offset_from_logical(grab_offset, scale)
                .ok_or(DragHostError::InvalidGrabOffset)?;
        let position = DockGeometry::physical_position_for_grab(cursor, physical_grab_offset);
        self.native_ops.move_window(active.native_window, position)?;
        Ok(())
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
            controller_cx.notify();
        });
        Ok(())
    }

    /// Returns the Dragging role for the current temporary window, if present.
    pub fn role_for_window_id(&self, window_id: WindowId) -> Option<DockWindowRole> {
        let active = self.active.as_ref()?;
        (active.window_id == window_id).then_some(DockWindowRole::Dragging { id: active.id })
    }

    /// Returns the active DragHost HWND for native drop-target exclusion.
    ///
    /// The drag/drop router must pass this value as `exclude` to
    /// `NativeWindowOps::window_from_point`. The platform resolver skips the
    /// temporary surface and continues through the desktop Z-order.
    pub(crate) fn active_native_window_id(&self) -> Option<NativeWindowId> {
        self.active.as_ref().map(|active| active.native_window)
    }

    /// Returns the identifier of the currently active DragHost, if present.
    pub fn active_drag_host_id(&self) -> Option<DragHostId> {
        self.active.as_ref().map(|active| active.id)
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
    #[error("Panel {0:?} has no cached view or matching active Panel drag session")]
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
            .size_full()
            .bg(rgb(self.theme.colors.window_background));

        if let Some(panel) = self.panel
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
