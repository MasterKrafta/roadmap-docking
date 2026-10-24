use dock_core::{DockCommand, DragHandle, DragPayload, PanelInstance};
use gpui::{AnyElement, ClickEvent, Context, MouseDownEvent, SharedString, Window};

use crate::host::{
    CommandClickHandler, DockHost, DragStartHandler, NoopHost, ResizeStartHandler,
};
use crate::main_window::MainWindow;
use crate::requests::{DockMenuRequest, ResizeHandle};

/// Routes command clicks through the owning window's DockController.
///
/// Menu state and gesture sessions are connected in their dedicated roadmap
/// stages, so those handlers still use NoopHost for now.
pub(crate) struct WindowHost<'a, 'cx> {
    cx: &'a Context<'cx, MainWindow>,
}

impl<'a, 'cx> WindowHost<'a, 'cx> {
    pub(crate) fn new(cx: &'a Context<'cx, MainWindow>) -> Self {
        Self { cx }
    }

    fn noop(&self) -> NoopHost {
        NoopHost
    }
}

impl DockHost for WindowHost<'_, '_> {
    fn panel_title(&self, instance: &PanelInstance) -> SharedString {
        self.noop().panel_title(instance)
    }

    fn panel_icon(&self, instance: &PanelInstance) -> &'static str {
        self.noop().panel_icon(instance)
    }

    fn render_panel(&mut self, instance: &PanelInstance) -> AnyElement {
        self.noop().render_panel(instance)
    }

    fn command(&self, command: DockCommand) -> CommandClickHandler {
        Box::new(self.cx.listener::<ClickEvent>(
            move |this, _event, _window, cx| {
                this.apply_dock_command(command.clone());
                cx.notify();
            },
        ))
    }

    fn menu(&self, request: DockMenuRequest) -> CommandClickHandler {
        self.noop().menu(request)
    }

    fn drag_start(&self, payload: DragPayload, handle: DragHandle) -> DragStartHandler {
        self.noop().drag_start(payload, handle)
    }

    fn resize_start(&self, handle: ResizeHandle) -> ResizeStartHandler {
        self.noop().resize_start(handle)
    }
}
