use dock_core::{DockCommand, DragHandle, DragPayload, FloatingGroupId, PanelInstance};
use gpui::{
    AnyElement, App, ClickEvent, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    SharedString, Window,
};

use crate::requests::{DockMenuRequest, ResizeHandle};

/// A GPUI click handler created by the host for a command or a menu request.
pub type CommandClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A keyboard-triggered dock command handler.
pub type CommandKeyHandler = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App) + 'static>;

/// Begins a Core drag session from a UI interaction surface.
pub type DragStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Begins a resize gesture from Core-provided geometry.
pub type ResizeStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Handles pointer movement shared by all active surface gestures.
pub type SurfaceMouseMoveHandler = Box<dyn Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static>;

/// Finishes the active pointer gesture when the left mouse button is released.
pub type SurfaceMouseUpHandler = Box<dyn Fn(&MouseUpEvent, &mut Window, &mut App) + 'static>;

/// Supplies panel presentation and converts UI interactions into host-owned requests.
///
/// The adapter only asks for handlers; it does not own the layout or apply commands.
pub trait DockHost {
    fn panel_title(&self, instance: &PanelInstance) -> SharedString;

    fn panel_icon(&self, instance: &PanelInstance) -> &'static str;

    fn render_panel(&mut self, instance: &PanelInstance) -> AnyElement;

    fn command(&self, command: DockCommand) -> CommandClickHandler;

    /// Handles keyboard activation for hosts that support dock commands.
    ///
    /// Preview hosts may keep the default no-op implementation.
    fn command_key(&self, _command: DockCommand) -> CommandKeyHandler {
        Box::new(|_, _, _| {})
    }

    fn close_floating(&self, group: FloatingGroupId) -> CommandClickHandler;

    fn menu(&self, request: DockMenuRequest) -> CommandClickHandler;

    fn drag_start(&self, payload: DragPayload, handle: DragHandle) -> DragStartHandler;

    fn resize_start(&self, handle: ResizeHandle) -> ResizeStartHandler;

    /// Installs the single root-level move listener for the active surface gesture.
    fn surface_mouse_move(&self) -> SurfaceMouseMoveHandler {
        Box::new(|_, _, _| {})
    }

    /// Installs the single root-level release listener for the active surface gesture.
    fn surface_mouse_up(&self) -> SurfaceMouseUpHandler {
        Box::new(|_, _, _| {})
    }
}

/// A host with the standard panel registry and no-op event handlers.
///
/// This is suitable for preview windows and component/layout examples.
#[derive(Default)]
pub struct NoopHost;

impl DockHost for NoopHost {
    fn panel_title(&self, instance: &PanelInstance) -> SharedString {
        crate::panels::panel_title(instance).into()
    }

    fn panel_icon(&self, instance: &PanelInstance) -> &'static str {
        crate::panels::panel_icon(instance)
    }

    fn render_panel(&mut self, instance: &PanelInstance) -> AnyElement {
        crate::panels::render_panel(instance)
    }

    fn command(&self, _command: DockCommand) -> CommandClickHandler {
        Box::new(|_, _, _| {})
    }

    fn command_key(&self, _command: DockCommand) -> CommandKeyHandler {
        Box::new(|_, _, _| {})
    }

    fn close_floating(&self, _group: FloatingGroupId) -> CommandClickHandler {
        Box::new(|_, _, _| {})
    }

    fn menu(&self, _request: DockMenuRequest) -> CommandClickHandler {
        Box::new(|_, _, _| {})
    }

    fn drag_start(&self, _payload: DragPayload, _handle: DragHandle) -> DragStartHandler {
        Box::new(|_, _, _| {})
    }

    fn resize_start(&self, _handle: ResizeHandle) -> ResizeStartHandler {
        Box::new(|_, _, _| {})
    }
}
